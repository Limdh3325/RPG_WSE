use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::PersistenceError;
use crate::lease::verify_lease_fencing_tx;
use crate::models::{
    CommandReceiptRecord, DurableIntent, DurableOperation, PersistenceAck, ReceiptStatus,
};

#[derive(Clone)]
pub struct DurableWriter {
    pool: PgPool,
    admitting: Arc<AtomicBool>,
    failpoints: Arc<Mutex<HashSet<String>>>,
}

impl DurableWriter {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            admitting: Arc::new(AtomicBool::new(true)),
            failpoints: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn set_admitting(&self, admitting: bool) {
        self.admitting.store(admitting, Ordering::SeqCst);
    }

    pub fn is_admitting(&self) -> bool {
        self.admitting.load(Ordering::SeqCst)
    }

    pub async fn set_failpoint(&self, name: impl Into<String>) {
        self.failpoints.lock().await.insert(name.into());
    }

    pub async fn clear_failpoint(&self, name: &str) {
        self.failpoints.lock().await.remove(name);
    }

    pub async fn has_failpoint(&self, name: &str) -> bool {
        self.failpoints.lock().await.contains(name)
    }

    pub async fn query_receipt(
        &self,
        command_id: Uuid,
    ) -> Result<Option<CommandReceiptRecord>, PersistenceError> {
        let row = sqlx::query(
            r#"
            SELECT command_id, world_id, region_id, actor_id, status, payload_hash, committed_tick, result_json
            FROM command_receipts
            WHERE command_id = $1
            "#,
        )
        .bind(command_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            let status_str: String = r.get("status");
            let status =
                ReceiptStatus::from_str_loose(&status_str).unwrap_or(ReceiptStatus::Pending);

            Ok(Some(CommandReceiptRecord {
                command_id: r.get("command_id"),
                world_id: r.get("world_id"),
                region_id: r.get("region_id"),
                actor_id: r.get("actor_id"),
                status,
                payload_hash: r.get("payload_hash"),
                committed_tick: r.get("committed_tick"),
                result_json: r.get("result_json"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn commit_intent(
        &self,
        intent: DurableIntent,
    ) -> Result<PersistenceAck, PersistenceError> {
        if !self.is_admitting() {
            return Err(PersistenceError::AdmissionPaused);
        }

        if self.has_failpoint("before_begin").await {
            return Err(PersistenceError::FailpointInjected("before_begin".into()));
        }

        let mut tx = self.pool.begin().await?;

        // 1. Lease / Epoch fencing
        verify_lease_fencing_tx(&mut tx, intent.region_id, intent.ownership_epoch).await?;

        // 2. Receipt check / idempotency lock
        let existing = sqlx::query(
            r#"
            SELECT status, payload_hash
            FROM command_receipts
            WHERE command_id = $1
            FOR UPDATE
            "#,
        )
        .bind(intent.command_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = existing {
            let stored_hash: Vec<u8> = row.get("payload_hash");
            if stored_hash != intent.payload_hash {
                return Err(PersistenceError::CommandHashMismatch {
                    command_id: intent.command_id,
                });
            }

            let status_str: String = row.get("status");
            let status =
                ReceiptStatus::from_str_loose(&status_str).unwrap_or(ReceiptStatus::Pending);

            match status {
                ReceiptStatus::Succeeded => {
                    return Ok(PersistenceAck {
                        command_id: intent.command_id,
                        region_id: intent.region_id,
                        status: ReceiptStatus::Succeeded,
                        committed_seq: None,
                        error_code: None,
                    });
                }
                ReceiptStatus::Rejected | ReceiptStatus::Failed => {
                    return Ok(PersistenceAck {
                        command_id: intent.command_id,
                        region_id: intent.region_id,
                        status,
                        committed_seq: None,
                        error_code: Some("PREVIOUSLY_REJECTED".into()),
                    });
                }
                ReceiptStatus::Pending => {
                    // Previous execution timed out or crashed mid-flight; proceed with reconciliation
                }
            }
        } else {
            sqlx::query(
                r#"
                INSERT INTO command_receipts (
                    command_id, world_id, region_id, actor_id, status,
                    payload_hash, committed_tick, result_json, created_at
                )
                VALUES ($1, $2, $3, $4, 'PENDING', $5, $6, '{}'::jsonb, clock_timestamp())
                "#,
            )
            .bind(intent.command_id)
            .bind(intent.world_id)
            .bind(intent.region_id)
            .bind(intent.actor_id)
            .bind(&intent.payload_hash[..])
            .bind(intent.tick)
            .execute(&mut *tx)
            .await?;
        }

        // 3. Asset reservation with canonical lock ordering to prevent deadlocks
        self.apply_operations(&mut tx, &intent.operations).await?;

        // 4. Sequence allocation and domain events emission
        let mut current_seq: i64 = {
            let row = sqlx::query(
                r#"
                SELECT durable_event_seq
                FROM regions
                WHERE id = $1
                FOR UPDATE
                "#,
            )
            .bind(intent.region_id)
            .fetch_one(&mut *tx)
            .await?;
            row.get("durable_event_seq")
        };

        for ev in &intent.emitted_events {
            current_seq += 1;

            sqlx::query(
                r#"
                INSERT INTO domain_events (
                    region_id, region_event_seq, event_id, world_id, command_id,
                    event_type, schema_version, rules_version, tick,
                    causation_event_id, correlation_id, payload, created_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, clock_timestamp())
                "#,
            )
            .bind(intent.region_id)
            .bind(current_seq)
            .bind(ev.event_id)
            .bind(intent.world_id)
            .bind(intent.command_id)
            .bind(&ev.event_type)
            .bind(ev.schema_version)
            .bind(ev.rules_version)
            .bind(ev.tick)
            .bind(ev.causation_event_id)
            .bind(ev.correlation_id)
            .bind(&ev.payload)
            .execute(&mut *tx)
            .await?;

            if let Some(ref topic) = ev.outbox_topic {
                sqlx::query(
                    r#"
                    INSERT INTO outbox (
                        id, event_id, topic, payload, attempts, next_attempt_at, created_at
                    )
                    VALUES (gen_random_uuid(), $1, $2, $3, 0, clock_timestamp(), clock_timestamp())
                    "#,
                )
                .bind(ev.event_id)
                .bind(topic)
                .bind(&ev.payload)
                .execute(&mut *tx)
                .await?;
            }
        }

        // Update region's durable event sequence
        sqlx::query(
            r#"
            UPDATE regions
            SET durable_event_seq = $1
            WHERE id = $2
            "#,
        )
        .bind(current_seq)
        .bind(intent.region_id)
        .execute(&mut *tx)
        .await?;

        // 5. Update receipt status to SUCCEEDED
        sqlx::query(
            r#"
            UPDATE command_receipts
            SET status = 'SUCCEEDED', committed_tick = $2, completed_at = clock_timestamp()
            WHERE command_id = $1
            "#,
        )
        .bind(intent.command_id)
        .bind(intent.tick)
        .execute(&mut *tx)
        .await?;

        if self.has_failpoint("fail_pre_commit").await || self.has_failpoint("before_commit").await
        {
            return Err(PersistenceError::FailpointInjected("before_commit".into()));
        }

        tx.commit().await?;

        if self.has_failpoint("after_commit_before_ack").await {
            return Err(PersistenceError::FailpointInjected(
                "after_commit_before_ack".into(),
            ));
        }

        Ok(PersistenceAck {
            command_id: intent.command_id,
            region_id: intent.region_id,
            status: ReceiptStatus::Succeeded,
            committed_seq: Some(current_seq),
            error_code: None,
        })
    }

    async fn apply_operations(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        operations: &[DurableOperation],
    ) -> Result<(), PersistenceError> {
        // Collect and sort wallets by entity_id to eliminate deadlock risk
        let mut wallet_ids = Vec::new();
        let mut market_keys = Vec::new();

        for op in operations {
            match op {
                DurableOperation::WalletTransfer { wallet_id, .. } => {
                    wallet_ids.push(*wallet_id);
                }
                DurableOperation::MarketPurchase {
                    settlement_id,
                    buyer_wallet_id,
                    seller_wallet_id,
                    sku,
                    ..
                } => {
                    wallet_ids.push(*buyer_wallet_id);
                    if let Some(sid) = seller_wallet_id {
                        wallet_ids.push(*sid);
                    }
                    market_keys.push((*settlement_id, sku.clone()));
                }
                DurableOperation::InventoryTransfer { .. } => {}
            }
        }

        wallet_ids.sort();
        wallet_ids.dedup();

        market_keys.sort();
        market_keys.dedup();

        // Lock wallets in deterministic order
        for wid in &wallet_ids {
            sqlx::query(
                r#"
                SELECT gold, version
                FROM wallets
                WHERE entity_id = $1
                FOR UPDATE
                "#,
            )
            .bind(wid)
            .fetch_one(&mut **tx)
            .await?;
        }

        // Lock market stock in deterministic order
        for (sid, sku) in &market_keys {
            sqlx::query(
                r#"
                SELECT quantity, version
                FROM market_stock
                WHERE settlement_id = $1 AND sku = $2
                FOR UPDATE
                "#,
            )
            .bind(sid)
            .bind(sku)
            .fetch_one(&mut **tx)
            .await?;
        }

        // Now apply mutations with safety checks
        for op in operations {
            match op {
                DurableOperation::WalletTransfer { wallet_id, delta } => {
                    let (gold,): (i64,) = sqlx::query_as(
                        r#"
                        SELECT gold FROM wallets WHERE entity_id = $1
                        "#,
                    )
                    .bind(wallet_id)
                    .fetch_one(&mut **tx)
                    .await?;

                    let new_gold = gold.checked_add(*delta).ok_or_else(|| {
                        PersistenceError::InvalidOperation(
                            "Wallet balance arithmetic overflow".into(),
                        )
                    })?;

                    if new_gold < 0 {
                        return Err(PersistenceError::InsufficientFunds {
                            wallet_id: *wallet_id,
                            balance: gold,
                            required: -delta,
                        });
                    }

                    sqlx::query(
                        r#"
                        UPDATE wallets
                        SET gold = gold + $2, version = version + 1
                        WHERE entity_id = $1
                        "#,
                    )
                    .bind(wallet_id)
                    .bind(delta)
                    .execute(&mut **tx)
                    .await?;
                }
                DurableOperation::MarketPurchase {
                    settlement_id,
                    buyer_wallet_id,
                    seller_wallet_id,
                    sku,
                    quantity,
                    unit_price,
                } => {
                    if *quantity <= 0 || *unit_price <= 0 {
                        return Err(PersistenceError::InvalidOperation(
                            "Purchase quantity and unit_price must be strictly positive".into(),
                        ));
                    }

                    let total_cost = quantity.checked_mul(*unit_price).ok_or_else(|| {
                        PersistenceError::InvalidOperation("Purchase total cost overflow".into())
                    })?;

                    // Check buyer balance
                    let (gold,): (i64,) = sqlx::query_as(
                        r#"
                        SELECT gold FROM wallets WHERE entity_id = $1
                        "#,
                    )
                    .bind(buyer_wallet_id)
                    .fetch_one(&mut **tx)
                    .await?;

                    if gold < total_cost {
                        return Err(PersistenceError::InsufficientFunds {
                            wallet_id: *buyer_wallet_id,
                            balance: gold,
                            required: total_cost,
                        });
                    }

                    // Check stock
                    let (stock,): (i64,) = sqlx::query_as(
                        r#"
                        SELECT quantity FROM market_stock WHERE settlement_id = $1 AND sku = $2
                        "#,
                    )
                    .bind(settlement_id)
                    .bind(sku)
                    .fetch_one(&mut **tx)
                    .await?;

                    if stock < *quantity {
                        return Err(PersistenceError::InsufficientStock {
                            settlement_id: *settlement_id,
                            sku: sku.clone(),
                            available: stock,
                            required: *quantity,
                        });
                    }

                    // Deduct gold from buyer
                    sqlx::query(
                        r#"
                        UPDATE wallets
                        SET gold = gold - $2, version = version + 1
                        WHERE entity_id = $1
                        "#,
                    )
                    .bind(buyer_wallet_id)
                    .bind(total_cost)
                    .execute(&mut **tx)
                    .await?;

                    // Credit seller if specified
                    if let Some(seller_id) = seller_wallet_id {
                        let (seller_gold,): (i64,) = sqlx::query_as(
                            r#"
                            SELECT gold FROM wallets WHERE entity_id = $1
                            "#,
                        )
                        .bind(seller_id)
                        .fetch_one(&mut **tx)
                        .await?;

                        seller_gold.checked_add(total_cost).ok_or_else(|| {
                            PersistenceError::InvalidOperation(
                                "Seller wallet arithmetic overflow".into(),
                            )
                        })?;

                        sqlx::query(
                            r#"
                            UPDATE wallets
                            SET gold = gold + $2, version = version + 1
                            WHERE entity_id = $1
                            "#,
                        )
                        .bind(seller_id)
                        .bind(total_cost)
                        .execute(&mut **tx)
                        .await?;
                    }

                    // Deduct stock
                    sqlx::query(
                        r#"
                        UPDATE market_stock
                        SET quantity = quantity - $3, version = version + 1
                        WHERE settlement_id = $1 AND sku = $2
                        "#,
                    )
                    .bind(settlement_id)
                    .bind(sku)
                    .bind(quantity)
                    .execute(&mut **tx)
                    .await?;

                    // Upsert inventory
                    sqlx::query(
                        r#"
                        INSERT INTO inventory_slots (owner_entity_id, sku, quantity, version)
                        VALUES ($1, $2, $3, 1)
                        ON CONFLICT (owner_entity_id, sku)
                        DO UPDATE SET quantity = inventory_slots.quantity + $3, version = inventory_slots.version + 1
                        "#,
                    )
                    .bind(buyer_wallet_id)
                    .bind(sku)
                    .bind(quantity)
                    .execute(&mut **tx)
                    .await?;
                }
                DurableOperation::InventoryTransfer {
                    owner_entity_id,
                    sku,
                    quantity_delta,
                } => {
                    let existing: Option<(i64,)> = sqlx::query_as(
                        r#"
                        SELECT quantity FROM inventory_slots WHERE owner_entity_id = $1 AND sku = $2 FOR UPDATE
                        "#,
                    )
                    .bind(owner_entity_id)
                    .bind(sku)
                    .fetch_optional(&mut **tx)
                    .await?;

                    let current_qty = existing.map(|(q,)| q).unwrap_or(0);
                    let new_qty = current_qty.checked_add(*quantity_delta).ok_or_else(|| {
                        PersistenceError::InvalidOperation("Inventory quantity overflow".into())
                    })?;

                    if new_qty < 0 {
                        return Err(PersistenceError::InsufficientStock {
                            settlement_id: Uuid::nil(),
                            sku: sku.clone(),
                            available: current_qty,
                            required: -quantity_delta,
                        });
                    }

                    sqlx::query(
                        r#"
                        INSERT INTO inventory_slots (owner_entity_id, sku, quantity, version)
                        VALUES ($1, $2, $3, 1)
                        ON CONFLICT (owner_entity_id, sku)
                        DO UPDATE SET quantity = inventory_slots.quantity + $3, version = inventory_slots.version + 1
                        "#,
                    )
                    .bind(owner_entity_id)
                    .bind(sku)
                    .bind(quantity_delta)
                    .execute(&mut **tx)
                    .await?;
                }
            }
        }

        Ok(())
    }
}
