use crate::error::PersistenceError;
use crate::lease::verify_lease_fencing_tx;
use crate::models::{
    CommandReceiptRecord, DurableIntent, DurableOperation, PersistenceAck, ReceiptStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Failpoint {
    #[default]
    None,
    BeforeBegin,
    BeforeAssetUpdate,
    AfterAssetUpdateBeforeCommit,
    AfterCommitBeforeAck,
}

#[derive(sqlx::FromRow)]
struct ExistingReceiptRow {
    status: String,
    payload_hash: Vec<u8>,
    result_json: Option<serde_json::Value>,
    committed_tick: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct FullReceiptRow {
    command_id: Uuid,
    world_id: Uuid,
    region_id: Uuid,
    actor_id: Uuid,
    payload_hash: Vec<u8>,
    status: String,
    committed_tick: Option<i64>,
    result_json: Option<serde_json::Value>,
}

pub struct DurableTransactionWriter;

impl DurableTransactionWriter {
    pub async fn commit_intent(
        pool: &PgPool,
        intent: &DurableIntent,
        failpoint: Failpoint,
    ) -> Result<PersistenceAck, PersistenceError> {
        // Failpoint 1: Before BEGIN (TEST-PER-001)
        if failpoint == Failpoint::BeforeBegin {
            return Err(PersistenceError::FailpointInjected(
                "Injected failure before BEGIN".to_string(),
            ));
        }

        let mut tx = pool.begin().await?;

        // 1. Lease fencing check (FOR UPDATE lock on region_leases)
        verify_lease_fencing_tx(&mut tx, intent.region_id, intent.ownership_epoch).await?;

        // 2. Command Receipt & Idempotency check
        let existing_receipt: Option<ExistingReceiptRow> = sqlx::query_as(
            r#"
            SELECT status, payload_hash, result_json, committed_tick
            FROM command_receipts
            WHERE command_id = $1
            FOR UPDATE
            "#,
        )
        .bind(intent.command_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = existing_receipt {
            if row.payload_hash != intent.payload_hash {
                return Err(PersistenceError::CommandHashMismatch {
                    command_id: intent.command_id,
                });
            }

            if row.status == ReceiptStatus::Succeeded.as_str() {
                // Idempotent hit: already committed. Return existing ACK without mutating state.
                return Ok(PersistenceAck {
                    command_id: intent.command_id,
                    region_id: intent.region_id,
                    status: ReceiptStatus::Succeeded,
                    committed_seq: row.committed_tick,
                    result_json: row.result_json.unwrap_or_else(|| {
                        serde_json::json!({
                            "status": "ALREADY_COMMITTED",
                            "tick": row.committed_tick
                        })
                    }),
                });
            }
        } else {
            // First appearance: insert pending receipt
            sqlx::query(
                r#"
                INSERT INTO command_receipts (command_id, world_id, region_id, actor_id, payload_hash, status, committed_tick, created_at)
                VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, clock_timestamp())
                "#,
            )
            .bind(intent.command_id)
            .bind(intent.world_id)
            .bind(intent.region_id)
            .bind(intent.actor_id)
            .bind(&intent.payload_hash[..])
            .bind(intent.tick as i64)
            .execute(&mut *tx)
            .await?;
        }

        // Failpoint: Before Asset Update
        if failpoint == Failpoint::BeforeAssetUpdate {
            return Err(PersistenceError::FailpointInjected(
                "Injected failure before asset update".to_string(),
            ));
        }

        // 3. Asset Operations in single transaction
        for op in &intent.operations {
            match op {
                DurableOperation::WalletTransfer { wallet_id, delta } => {
                    let wallet: Option<(i64, i64)> = sqlx::query_as(
                        r#"
                        SELECT currency_balance, version
                        FROM wallets
                        WHERE id = $1
                        FOR UPDATE
                        "#,
                    )
                    .bind(wallet_id)
                    .fetch_optional(&mut *tx)
                    .await?;

                    let (current_balance, _) = wallet.ok_or_else(|| {
                        PersistenceError::NotFound(format!("Wallet {wallet_id} not found"))
                    })?;

                    let new_balance = current_balance.checked_add(*delta).ok_or_else(|| {
                        PersistenceError::InvalidOperation("Wallet balance overflow".to_string())
                    })?;

                    if new_balance < 0 {
                        return Err(PersistenceError::InsufficientFunds {
                            wallet_id: *wallet_id,
                            balance: current_balance,
                            required: -delta,
                        });
                    }

                    sqlx::query(
                        r#"
                        UPDATE wallets
                        SET currency_balance = $2,
                            version = version + 1,
                            updated_at = clock_timestamp()
                        WHERE id = $1
                        "#,
                    )
                    .bind(wallet_id)
                    .bind(new_balance)
                    .execute(&mut *tx)
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
                    let total_cost = quantity.checked_mul(*unit_price).ok_or_else(|| {
                        PersistenceError::InvalidOperation("Total cost overflow".to_string())
                    })?;

                    // Deduct Buyer Wallet
                    let buyer_wallet: Option<(i64, i64)> = sqlx::query_as(
                        r#"
                        SELECT currency_balance, version
                        FROM wallets
                        WHERE id = $1
                        FOR UPDATE
                        "#,
                    )
                    .bind(buyer_wallet_id)
                    .fetch_optional(&mut *tx)
                    .await?;

                    let (buyer_balance, _) = buyer_wallet.ok_or_else(|| {
                        PersistenceError::NotFound(format!(
                            "Buyer wallet {buyer_wallet_id} not found"
                        ))
                    })?;

                    if buyer_balance < total_cost {
                        return Err(PersistenceError::InsufficientFunds {
                            wallet_id: *buyer_wallet_id,
                            balance: buyer_balance,
                            required: total_cost,
                        });
                    }

                    sqlx::query(
                        r#"
                        UPDATE wallets
                        SET currency_balance = currency_balance - $2,
                            version = version + 1,
                            updated_at = clock_timestamp()
                        WHERE id = $1
                        "#,
                    )
                    .bind(buyer_wallet_id)
                    .bind(total_cost)
                    .execute(&mut *tx)
                    .await?;

                    // Credit Seller Wallet (or Treasury if specified)
                    if let Some(seller_id) = seller_wallet_id {
                        sqlx::query(
                            r#"
                            UPDATE wallets
                            SET currency_balance = currency_balance + $2,
                                version = version + 1,
                                updated_at = clock_timestamp()
                            WHERE id = $1
                            "#,
                        )
                        .bind(seller_id)
                        .bind(total_cost)
                        .execute(&mut *tx)
                        .await?;
                    } else {
                        sqlx::query(
                            r#"
                            UPDATE settlements
                            SET treasury_balance = treasury_balance + $2
                            WHERE id = $1
                            "#,
                        )
                        .bind(settlement_id)
                        .bind(total_cost)
                        .execute(&mut *tx)
                        .await?;
                    }

                    // Deduct Market Stock
                    let stock: Option<(i64, i64, i64)> = sqlx::query_as(
                        r#"
                        SELECT quantity, price, version
                        FROM market_stock
                        WHERE settlement_id = $1 AND sku = $2
                        FOR UPDATE
                        "#,
                    )
                    .bind(settlement_id)
                    .bind(sku)
                    .fetch_optional(&mut *tx)
                    .await?;

                    let (available_stock, _, _) = stock.ok_or_else(|| {
                        PersistenceError::NotFound(format!(
                            "Market stock for settlement {settlement_id}, sku {sku} not found"
                        ))
                    })?;

                    if available_stock < *quantity {
                        return Err(PersistenceError::InsufficientStock {
                            settlement_id: *settlement_id,
                            sku: sku.clone(),
                            available: available_stock,
                            required: *quantity,
                        });
                    }

                    sqlx::query(
                        r#"
                        UPDATE market_stock
                        SET quantity = quantity - $3,
                            version = version + 1,
                            updated_at = clock_timestamp()
                        WHERE settlement_id = $1 AND sku = $2
                        "#,
                    )
                    .bind(settlement_id)
                    .bind(sku)
                    .bind(quantity)
                    .execute(&mut *tx)
                    .await?;
                }

                DurableOperation::InventoryTransfer {
                    entity_id,
                    slot_index,
                    item_id,
                    sku,
                    quantity,
                    durability,
                } => {
                    sqlx::query(
                        r#"
                        INSERT INTO inventory_slots (entity_id, slot_index, item_id, sku, quantity, durability, updated_at)
                        VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp())
                        ON CONFLICT (entity_id, slot_index) DO UPDATE
                        SET item_id = EXCLUDED.item_id,
                            sku = EXCLUDED.sku,
                            quantity = EXCLUDED.quantity,
                            durability = EXCLUDED.durability,
                            updated_at = clock_timestamp()
                        "#,
                    )
                    .bind(entity_id)
                    .bind(slot_index)
                    .bind(item_id)
                    .bind(sku)
                    .bind(quantity)
                    .bind(durability)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        // Failpoint 2: After Asset Update before COMMIT (TEST-PER-002)
        if failpoint == Failpoint::AfterAssetUpdateBeforeCommit {
            return Err(PersistenceError::FailpointInjected(
                "Injected crash after asset update before commit".to_string(),
            ));
        }

        // 4. Domain Events & Outbox in same transaction
        let mut latest_seq = 0i64;
        for ev in &intent.emitted_events {
            let row: (i64,) = sqlx::query_as(
                r#"
                UPDATE regions
                SET durable_event_seq = durable_event_seq + 1
                WHERE id = $1
                RETURNING durable_event_seq
                "#,
            )
            .bind(intent.region_id)
            .fetch_one(&mut *tx)
            .await?;

            latest_seq = row.0;

            let payload_bytes = serde_json::to_vec(&ev.payload).map_err(|e| {
                PersistenceError::InvalidOperation(format!("Payload serialization failed: {e}"))
            })?;

            sqlx::query(
                r#"
                INSERT INTO domain_events (id, world_id, region_id, event_seq, stream_type, aggregate_id, event_type, payload_json, occurred_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, clock_timestamp())
                "#,
            )
            .bind(ev.id)
            .bind(intent.world_id)
            .bind(intent.region_id)
            .bind(latest_seq)
            .bind(&ev.stream_type)
            .bind(ev.aggregate_id)
            .bind(&ev.event_type)
            .bind(&ev.payload)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                r#"
                INSERT INTO outbox (event_id, world_id, region_id, topic, payload_bytes, payload_json, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp())
                "#,
            )
            .bind(ev.id)
            .bind(intent.world_id)
            .bind(intent.region_id)
            .bind(&ev.topic)
            .bind(&payload_bytes)
            .bind(&ev.payload)
            .execute(&mut *tx)
            .await?;
        }

        // 5. Finalize Command Receipt in same transaction
        let result_json = serde_json::json!({
            "status": "COMMITTED",
            "tick": intent.tick,
            "event_seq": latest_seq
        });

        sqlx::query(
            r#"
            UPDATE command_receipts
            SET status = 'SUCCEEDED',
                committed_tick = $1,
                result_json = $2,
                completed_at = clock_timestamp()
            WHERE command_id = $3
            "#,
        )
        .bind(intent.tick as i64)
        .bind(&result_json)
        .bind(intent.command_id)
        .execute(&mut *tx)
        .await?;

        // 6. Commit single atomic transaction
        tx.commit().await?;

        // Failpoint 3: After Commit before ACK (TEST-PER-003)
        if failpoint == Failpoint::AfterCommitBeforeAck {
            return Err(PersistenceError::FailpointInjected(
                "Injected ACK drop after commit".to_string(),
            ));
        }

        Ok(PersistenceAck {
            command_id: intent.command_id,
            region_id: intent.region_id,
            status: ReceiptStatus::Succeeded,
            committed_seq: Some(intent.tick as i64),
            result_json,
        })
    }

    pub async fn lookup_receipt(
        pool: &PgPool,
        command_id: Uuid,
    ) -> Result<Option<CommandReceiptRecord>, PersistenceError> {
        let row: Option<FullReceiptRow> = sqlx::query_as(
            r#"
            SELECT command_id, world_id, region_id, actor_id, payload_hash, status, committed_tick, result_json
            FROM command_receipts
            WHERE command_id = $1
            "#,
        )
        .bind(command_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| CommandReceiptRecord {
            command_id: r.command_id,
            world_id: r.world_id,
            region_id: r.region_id,
            actor_id: r.actor_id,
            payload_hash: r.payload_hash,
            status: ReceiptStatus::from_str_loose(&r.status).unwrap_or(ReceiptStatus::Pending),
            committed_tick: r.committed_tick,
            result_json: r.result_json,
        }))
    }
}

pub async fn lookup_receipt(
    pool: &PgPool,
    command_id: Uuid,
) -> Result<Option<CommandReceiptRecord>, PersistenceError> {
    DurableTransactionWriter::lookup_receipt(pool, command_id).await
}
