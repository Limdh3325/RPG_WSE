use crate::error::PersistenceError;
use sqlx::{Acquire, PgPool};

pub const INITIAL_SCHEMA_SQL: &str = include_str!("../../../schemas/0001_initial.sql");

pub async fn apply_migration(pool: &PgPool) -> Result<(), PersistenceError> {
    // 1. Fast-path: Check if initial migration is already recorded
    let check: Result<Option<(String,)>, sqlx::Error> =
        sqlx::query_as("SELECT version FROM _schema_migrations WHERE version = $1")
            .bind("0001_initial")
            .fetch_optional(pool)
            .await;

    if let Ok(Some(_)) = check {
        return Ok(());
    }

    // 2. Slow-path: Acquire a dedicated connection and use transaction-scoped advisory lock.
    // Transaction-scoped lock (pg_advisory_xact_lock) is automatically released on commit/rollback,
    // guaranteeing no lock leakage across pooled connections.
    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;

    sqlx::query("SELECT pg_advisory_xact_lock(7142981)")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _schema_migrations (
            version VARCHAR(255) PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
        )
        "#,
    )
    .execute(&mut *tx)
    .await?;

    let applied: Option<(String,)> =
        sqlx::query_as("SELECT version FROM _schema_migrations WHERE version = $1 FOR UPDATE")
            .bind("0001_initial")
            .fetch_optional(&mut *tx)
            .await?;

    if applied.is_none() {
        sqlx::raw_sql(INITIAL_SCHEMA_SQL).execute(&mut *tx).await?;

        sqlx::query("INSERT INTO _schema_migrations (version) VALUES ($1)")
            .bind("0001_initial")
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}
