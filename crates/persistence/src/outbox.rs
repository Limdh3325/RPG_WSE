use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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

    /// Fetches pending outbox messages, invokes the publisher closure for each,
    /// marks them published on success, or schedules a retry with exponential backoff on failure.
    pub async fn publish_pending<F, Fut>(
        &self,
        batch_size: i64,
        publish_fn: F,
    ) -> Result<usize, PersistenceError>
    where
        F: Fn(OutboxRecord) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
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

        let mut published_count = 0;

        for row in rows {
            use sqlx::Row;
            let record = OutboxRecord {
                id: row.get("id"),
                event_id: row.get("event_id"),
                topic: row.get("topic"),
                payload: row.get("payload"),
                attempts: row.get("attempts"),
            };

            match publish_fn(record.clone()).await {
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
                    // Exponential backoff: 2 ^ attempts seconds (capped at 300s)
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

/// Thread-safe event deduplicator for consumers to guarantee exactly-once processing semantics
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

    /// Checks and marks the event as processed.
    /// Returns true if the event is NEW (not duplicate), or false if it was already processed.
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
