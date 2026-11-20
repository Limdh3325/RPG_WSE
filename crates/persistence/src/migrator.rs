use crate::error::PersistenceError;
use sqlx::PgPool;

pub const INITIAL_SCHEMA_SQL: &str = include_str!("../../../schemas/0001_initial.sql");

pub async fn apply_migration(pool: &PgPool) -> Result<(), PersistenceError> {
    let mut tx = pool.begin().await?;
    // Advisory lock to serialize migrations across concurrent processes/tests
    sqlx::query("SELECT pg_advisory_xact_lock(7142981)")
        .execute(&mut *tx)
        .await?;

    sqlx::raw_sql(INITIAL_SCHEMA_SQL)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}
