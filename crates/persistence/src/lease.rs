use crate::error::PersistenceError;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub async fn acquire_or_renew_lease(
    pool: &PgPool,
    region_id: Uuid,
    owner_id: &str,
    ttl_seconds: i64,
) -> Result<i64, PersistenceError> {
    let mut tx = pool.begin().await?;

    // Lock existing lease row if present
    let existing: Option<(i64, bool, Option<String>)> = sqlx::query_as(
        r#"
        SELECT ownership_epoch, (lease_expires_at > clock_timestamp()) AS is_valid, owner_id
        FROM region_leases
        WHERE region_id = $1
        FOR UPDATE
        "#,
    )
    .bind(region_id)
    .fetch_optional(&mut *tx)
    .await?;

    let epoch = match existing {
        Some((current_epoch, is_valid, existing_owner)) => {
            if is_valid && existing_owner.as_deref() == Some(owner_id) {
                // Renew within active epoch by the same owner
                sqlx::query(
                    r#"
                    UPDATE region_leases
                    SET lease_expires_at = clock_timestamp() + ($2 * interval '1 second'),
                        updated_at = clock_timestamp()
                    WHERE region_id = $1
                    "#,
                )
                .bind(region_id)
                .bind(ttl_seconds)
                .execute(&mut *tx)
                .await?;
                current_epoch
            } else {
                // Expired or takeover by new owner: increment epoch fencing
                let next_epoch = current_epoch.saturating_add(1);
                sqlx::query(
                    r#"
                    UPDATE region_leases
                    SET owner_id = $2,
                        ownership_epoch = $3,
                        lease_expires_at = clock_timestamp() + ($4 * interval '1 second'),
                        updated_at = clock_timestamp()
                    WHERE region_id = $1
                    "#,
                )
                .bind(region_id)
                .bind(owner_id)
                .bind(next_epoch)
                .bind(ttl_seconds)
                .execute(&mut *tx)
                .await?;
                next_epoch
            }
        }
        None => {
            // New lease, initial epoch 1
            let initial_epoch: i64 = 1;
            sqlx::query(
                r#"
                INSERT INTO region_leases (region_id, owner_id, ownership_epoch, lease_expires_at, updated_at)
                VALUES ($1, $2, $3, clock_timestamp() + ($4 * interval '1 second'), clock_timestamp())
                "#,
            )
            .bind(region_id)
            .bind(owner_id)
            .bind(initial_epoch)
            .bind(ttl_seconds)
            .execute(&mut *tx)
            .await?;
            initial_epoch
        }
    };

    tx.commit().await?;
    Ok(epoch)
}

pub async fn verify_lease_fencing_tx(
    tx: &mut Transaction<'_, Postgres>,
    region_id: Uuid,
    expected_epoch: i64,
) -> Result<(), PersistenceError> {
    let lease_row: Option<(i64, bool)> = sqlx::query_as(
        r#"
        SELECT ownership_epoch, (lease_expires_at > clock_timestamp()) AS is_valid
        FROM region_leases
        WHERE region_id = $1
        FOR SHARE
        "#,
    )
    .bind(region_id)
    .fetch_optional(&mut **tx)
    .await?;

    match lease_row {
        Some((epoch, is_valid)) => {
            if epoch != expected_epoch || !is_valid {
                return Err(PersistenceError::LeaseFencingFailed {
                    region_id,
                    expected_epoch,
                    actual_epoch: Some(epoch),
                });
            }
            Ok(())
        }
        None => Err(PersistenceError::LeaseFencingFailed {
            region_id,
            expected_epoch,
            actual_epoch: None,
        }),
    }
}
