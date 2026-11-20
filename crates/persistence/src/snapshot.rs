use crate::error::PersistenceError;
use game_domain::RegionStateDto;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const CURRENT_ENGINE_VERSION: &str = "0.1.0";
pub const CURRENT_RULES_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledActionDto {
    pub id: Uuid,
    pub entity_id: Option<Uuid>,
    pub due_tick: u64,
    pub action_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDto {
    pub region_id: Uuid,
    pub tick: u64,
    pub committed_event_seq: u64,
    pub ownership_epoch: i64,
    pub schema_version: u32,
    pub engine_version: String,
    pub rules_version: u32,
    pub rng_seed: u64,
    pub rng_step: u64,
    pub state: RegionStateDto,
    pub scheduled_actions: Vec<ScheduledActionDto>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SnapshotRow {
    pub id: Uuid,
    pub region_id: Uuid,
    pub tick: i64,
    pub committed_event_seq: i64,
    pub ownership_epoch: i64,
    pub schema_version: i32,
    pub engine_version: String,
    pub rules_version: i32,
    pub sha256: Vec<u8>,
    pub payload: Vec<u8>,
}

pub struct SnapshotManager;

impl SnapshotManager {
    /// Serializes SnapshotDto, calculates its SHA-256 hash, and inserts a row into `region_snapshots`.
    pub async fn save_snapshot(pool: &PgPool, dto: &SnapshotDto) -> Result<Uuid, PersistenceError> {
        let snapshot_id = Uuid::new_v4();
        let payload =
            serde_json::to_vec(dto).map_err(|e| PersistenceError::Serialization(e.to_string()))?;

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let sha256_hash = hasher.finalize().to_vec();

        sqlx::query(
            r#"
            INSERT INTO region_snapshots (
                id, region_id, tick, committed_event_seq, ownership_epoch,
                schema_version, engine_version, rules_version, sha256, payload
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(snapshot_id)
        .bind(dto.region_id)
        .bind(dto.tick as i64)
        .bind(dto.committed_event_seq as i64)
        .bind(dto.ownership_epoch)
        .bind(dto.schema_version as i32)
        .bind(&dto.engine_version)
        .bind(dto.rules_version as i32)
        .bind(&sha256_hash)
        .bind(&payload)
        .execute(pool)
        .await?;

        Ok(snapshot_id)
    }

    /// Loads the latest snapshot that passes SHA-256 checksum verification, version compatibility,
    /// and JSON deserialization.
    /// If corrupted or incompatible, it logs a warning and falls back to the previous valid snapshot.
    pub async fn load_latest_verified_snapshot(
        pool: &PgPool,
        region_id: Uuid,
    ) -> Result<Option<(Uuid, SnapshotDto)>, PersistenceError> {
        let rows: Vec<SnapshotRow> = sqlx::query_as(
            r#"
            SELECT id, region_id, tick, committed_event_seq, ownership_epoch,
                   schema_version, engine_version, rules_version, sha256, payload
            FROM region_snapshots
            WHERE region_id = $1
            ORDER BY tick DESC, created_at DESC
            "#,
        )
        .bind(region_id)
        .fetch_all(pool)
        .await?;

        for row in rows {
            // 1. Verify SHA-256 checksum
            let mut hasher = Sha256::new();
            hasher.update(&row.payload);
            let calculated_hash = hasher.finalize().to_vec();

            if calculated_hash != row.sha256 {
                warn!(
                    snapshot_id = %row.id,
                    tick = row.tick,
                    "Snapshot failed SHA-256 checksum verification (corrupted); falling back to earlier snapshot"
                );
                continue;
            }

            // 2. Verify version compatibility with current runtime
            if row.schema_version != CURRENT_SCHEMA_VERSION as i32
                || row.rules_version != CURRENT_RULES_VERSION as i32
                || row.engine_version != CURRENT_ENGINE_VERSION
            {
                warn!(
                    snapshot_id = %row.id,
                    tick = row.tick,
                    schema_version = row.schema_version,
                    engine_version = %row.engine_version,
                    rules_version = row.rules_version,
                    "Snapshot has incompatible versions; falling back to earlier snapshot"
                );
                continue;
            }

            // 3. Attempt deserialization & internal version validation
            match serde_json::from_slice::<SnapshotDto>(&row.payload) {
                Ok(dto) => {
                    if dto.schema_version != CURRENT_SCHEMA_VERSION
                        || dto.rules_version != CURRENT_RULES_VERSION
                        || dto.engine_version != CURRENT_ENGINE_VERSION
                    {
                        warn!(
                            snapshot_id = %row.id,
                            tick = row.tick,
                            "Snapshot DTO internal versions incompatible; falling back to earlier snapshot"
                        );
                        continue;
                    }
                    return Ok(Some((row.id, dto)));
                }
                Err(err) => {
                    warn!(
                        snapshot_id = %row.id,
                        tick = row.tick,
                        error = %err,
                        "Snapshot payload failed JSON deserialization; falling back to earlier snapshot"
                    );
                    continue;
                }
            }
        }

        Ok(None)
    }
}
