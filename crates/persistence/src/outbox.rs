use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::PersistenceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxRecord {
    pub id: Uuid,
    pub event_id: Uuid,
    pub topic: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

#[derive(Clone)]
pub struct OutboxPublisher {
    pool: PgPool,
}

impl OutboxPublisher {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Fetches pending outbox messages by leasing them in a quick transaction,
    /// invokes the external publisher closure OUTSIDE of the DB transaction/row lock,
    /// and then records the outcome in a second short transaction.
    /// This prevents blocking database connection pools or holding row locks during external network delays.
    pub async fn publish_pending<F, Fut>(
        &self,
        batch_size: i64,
        publish_fn: F,
    ) -> Result<usize, PersistenceError>
    where
        F: Fn(OutboxRecord) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        // Phase 1: Claim items with a short lease without holding locks during publishing
        let claimed_records: Vec<OutboxRecord> = {
            let mut tx = self.pool.begin().await?;

            let rows = sqlx::query(
                r#"
                SELECT id, event_id, topic, payload, attempts
                FROM outbox
                WHERE published_at IS NULL AND next_attempt_at <= clock_timestamp()
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
                "#,
            )
            .bind(batch_size)
            .fetch_all(&mut *tx)
            .await?;

            let mut records = Vec::with_capacity(rows.len());

            for row in rows {
                let id: Uuid = row.get("id");
                let event_id: Uuid = row.get("event_id");
                let topic: String = row.get("topic");
                let payload: serde_json::Value = row.get("payload");
                let attempts: i32 = row.get("attempts");

                // Advance next_attempt_at by 30 seconds to serve as a durable in-flight claim lease
                sqlx::query(
                    r#"
                    UPDATE outbox
                    SET next_attempt_at = clock_timestamp() + interval '30 seconds'
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;

                records.push(OutboxRecord {
                    id,
                    event_id,
                    topic,
                    payload,
                    attempts,
                });
            }

            tx.commit().await?;
            records
        };

        if claimed_records.is_empty() {
            return Ok(0);
        }

        // Phase 2: Execute external publishing outside the DB transaction
        let mut results = Vec::with_capacity(claimed_records.len());
        for record in claimed_records {
            let res = publish_fn(record.clone()).await;
            results.push((record, res));
        }

        // Phase 3: Finalize status in DB in a quick commit
        let mut tx = self.pool.begin().await?;
        let mut published_count = 0;

        for (record, res) in results {
            match res {
                Ok(()) => {
                    sqlx::query(
                        r#"
                        UPDATE outbox
                        SET published_at = clock_timestamp()
                        WHERE id = $1
                        "#,
                    )
                    .bind(record.id)
                    .execute(&mut *tx)
                    .await?;
                    published_count += 1;
                }
                Err(_err) => {
                    let delay_sec = (1 << record.attempts.min(8)).max(1);
                    sqlx::query(
                        r#"
                        UPDATE outbox
                        SET attempts = attempts + 1,
                            next_attempt_at = clock_timestamp() + ($2 * interval '1 second')
                        WHERE id = $1
                        "#,
                    )
                    .bind(record.id)
                    .bind(delay_sec as i64)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        tx.commit().await?;
        Ok(published_count)
    }
}

/// Durable, PostgreSQL-backed consumer deduplication store
#[derive(Clone)]
pub struct SqlxEventDeduplicator {
    pool: PgPool,
    consumer_id: String,
}

impl SqlxEventDeduplicator {
    pub fn new(pool: PgPool, consumer_id: impl Into<String>) -> Self {
        Self {
            pool,
            consumer_id: consumer_id.into(),
        }
    }

    /// Attempts to record the event into durable storage.
    /// Returns Ok(true) if the event is NEW (claimed for processing).
    /// Returns Ok(false) if the event was ALREADY PROCESSED or claimed (duplicate).
    pub async fn process_if_new(&self, event_id: Uuid) -> Result<bool, PersistenceError> {
        let result = sqlx::query(
            r#"
            INSERT INTO consumer_inbox (consumer_id, event_id, processed_at)
            VALUES ($1, $2, clock_timestamp())
            ON CONFLICT (consumer_id, event_id) DO NOTHING
            "#,
        )
        .bind(&self.consumer_id)
        .bind(event_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Attempts to record the event into durable storage within an existing transaction.
    /// Returns Ok(true) if the event is NEW (claimed for processing).
    /// Returns Ok(false) if the event was ALREADY PROCESSED or claimed (duplicate).
    pub async fn process_if_new_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event_id: Uuid,
    ) -> Result<bool, PersistenceError> {
        let result = sqlx::query(
            r#"
            INSERT INTO consumer_inbox (consumer_id, event_id, processed_at)
            VALUES ($1, $2, clock_timestamp())
            ON CONFLICT (consumer_id, event_id) DO NOTHING
            "#,
        )
        .bind(&self.consumer_id)
        .bind(event_id)
        .execute(&mut **tx)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn is_duplicate(&self, event_id: Uuid) -> Result<bool, PersistenceError> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1 FROM consumer_inbox
            WHERE consumer_id = $1 AND event_id = $2
            "#,
        )
        .bind(&self.consumer_id)
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }
}

/// Thread-safe in-memory event deduplicator for fast transient deduplication
#[derive(Debug, Clone, Default)]
pub struct EventDeduplicator {
    processed: Arc<Mutex<HashSet<Uuid>>>,
}

impl EventDeduplicator {
    pub fn new() -> Self {
        Self {
            processed: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn process_if_new(&self, event_id: Uuid) -> bool {
        let mut set = self.processed.lock().await;
        set.insert(event_id)
    }

    pub async fn is_duplicate(&self, event_id: Uuid) -> bool {
        let set = self.processed.lock().await;
        set.contains(&event_id)
    }

    pub async fn processed_count(&self) -> usize {
        let set = self.processed.lock().await;
        set.len()
    }
}
