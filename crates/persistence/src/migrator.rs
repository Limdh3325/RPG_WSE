use crate::error::PersistenceError;
use sqlx::PgPool;

pub const INITIAL_SCHEMA_SQL: &str = include_str!("../../../schemas/0001_initial.sql");

pub async fn apply_migration(pool: &PgPool) -> Result<(), PersistenceError> {
    // 1. Fast-path: Check if migration record already exists
    let check: Result<Option<(String,)>, sqlx::Error> =
        sqlx::query_as("SELECT version FROM _schema_migrations WHERE version = $1")
            .bind("0001_initial")
            .fetch_optional(pool)
            .await;

    if let Ok(Some(_)) = check {
        return Ok(());
    }

    // 2. Slow-path: Serialize across all processes using session-level advisory lock
    sqlx::query("SELECT pg_advisory_lock(7142981)")
        .execute(pool)
        .await?;

    let res = apply_migration_locked(pool).await;

    // Always release the advisory lock
    let _ = sqlx::query("SELECT pg_advisory_unlock(7142981)")
        .execute(pool)
        .await;

    res
}

async fn apply_migration_locked(pool: &PgPool) -> Result<(), PersistenceError> {
    let mut tx = pool.begin().await?;

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
