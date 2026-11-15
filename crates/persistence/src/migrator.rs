use crate::error::PersistenceError;
use sqlx::PgPool;

pub const INITIAL_SCHEMA_SQL: &str = include_str!("../../../schemas/0001_initial.sql");

pub async fn apply_migration(pool: &PgPool) -> Result<(), PersistenceError> {
    sqlx::raw_sql(INITIAL_SCHEMA_SQL)
        .execute(pool)
        .await
        .map_err(PersistenceError::Sqlx)?;
    Ok(())
}
