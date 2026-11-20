use crate::error::PersistenceError;
use crate::lease::acquire_or_renew_lease;
use crate::snapshot::{ScheduledActionDto, SnapshotManager};
use game_domain::{EntityId, EntityStateDto, PositionDto, RegionId, RegionStateDto};
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveredRegionState {
    pub region_id: Uuid,
    pub epoch: i64,
    pub tick: u64,
    pub committed_event_seq: u64,
    pub rng_seed: u64,
    pub rng_step: u64,
    pub state: RegionStateDto,
    pub scheduled_actions: Vec<ScheduledActionDto>,
    pub replayed_events_count: usize,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DomainEventRow {
    pub id: Uuid,
    pub event_seq: i64,
    pub event_type: String,
    pub tick: i64,
    pub payload_json: serde_json::Value,
}

pub struct RecoveryManager;

impl RecoveryManager {
    /// Executes the full region recovery pipeline:
    /// 1. Acquire/renew lease with new epoch
    /// 2. Load latest verified snapshot (auto-fallback if corrupted or incompatible)
    /// 3. Replay committed domain events with event_seq > snapshot.committed_event_seq
    /// 4. Reconcile state via deterministic event reducer
    /// 5. Return clean RecoveredRegionState
    pub async fn recover_region(
        pool: &PgPool,
        region_id: Uuid,
        owner_id: &str,
        ttl_seconds: i64,
    ) -> Result<RecoveredRegionState, PersistenceError> {
        // Step 1: Acquire/renew lease to ensure single-writer authority
        let new_epoch = acquire_or_renew_lease(pool, region_id, owner_id, ttl_seconds).await?;

        // Step 2: Load latest verified snapshot
        let verified_snapshot =
            SnapshotManager::load_latest_verified_snapshot(pool, region_id).await?;

        // Query authoritative region sequence from DB
        let db_seq: i64 = {
            use sqlx::Row;
            let row = sqlx::query("SELECT durable_event_seq FROM regions WHERE id = $1")
                .bind(region_id)
                .fetch_one(pool)
                .await?;
            row.get("durable_event_seq")
        };

        let (
            mut tick,
            mut committed_event_seq,
            rng_seed,
            mut rng_step,
            mut state,
            mut scheduled_actions,
        ) = if let Some((snapshot_id, snapshot)) = verified_snapshot {
            if snapshot.committed_event_seq as i64 > db_seq {
                return Err(PersistenceError::CorruptedSnapshot {
                    snapshot_id,
                    reason: format!(
                        "Snapshot watermark {} exceeds DB durable_event_seq {}",
                        snapshot.committed_event_seq, db_seq
                    ),
                });
            }

            info!(
                snapshot_id = %snapshot_id,
                region_id = %region_id,
                tick = snapshot.tick,
                event_seq = snapshot.committed_event_seq,
                "Restored base state from verified snapshot"
            );
            (
                snapshot.tick,
                snapshot.committed_event_seq,
                snapshot.rng_seed,
                snapshot.rng_step,
                snapshot.state,
                snapshot.scheduled_actions,
            )
        } else {
            info!(
                region_id = %region_id,
                "No valid snapshot found; initializing fresh base state"
            );
            (
                0,
                0,
                12345,
                0,
                RegionStateDto {
                    region_id: RegionId(region_id),
                    epoch: new_epoch as u64,
                    tick: 0,
                    status: game_domain::RegionStatus::Active,
                    entities: Vec::new(),
                },
                Vec::new(),
            )
        };

        // Always update the state epoch to the freshly acquired lease epoch
        state.epoch = new_epoch as u64;

        // Step 3: Query committed domain events > snapshot event_seq
        let unapplied_events: Vec<DomainEventRow> = sqlx::query_as(
            r#"
            SELECT event_id AS id, region_event_seq AS event_seq, event_type, tick, payload AS payload_json
            FROM domain_events
            WHERE region_id = $1 AND region_event_seq > $2
            ORDER BY region_event_seq ASC
            "#,
        )
        .bind(region_id)
        .bind(committed_event_seq as i64)
        .fetch_all(pool)
        .await?;

        let replayed_events_count = unapplied_events.len();

        // Step 4: Replay events into state using deterministic reducer
        let mut expected_seq = committed_event_seq + 1;
        for event in unapplied_events {
            let ev_seq = event.event_seq as u64;
            if ev_seq != expected_seq {
                return Err(PersistenceError::RecoveryFailed(format!(
                    "Non-contiguous event sequence during replay: expected {}, found {}",
                    expected_seq, ev_seq
                )));
            }
            expected_seq += 1;
            committed_event_seq = ev_seq;
            tick = tick.max(event.tick as u64);

            match event.event_type.as_str() {
                "ENTITY_TICK" => {
                    rng_step = rng_step.saturating_add(1);
                }
                "ENTITY_SPAWNED" => {
                    let entity_dto =
                        serde_json::from_value::<EntityStateDto>(event.payload_json.clone())
                            .map_err(|e| {
                                PersistenceError::RecoveryFailed(format!(
                                    "Failed to parse ENTITY_SPAWNED payload: {e}"
                                ))
                            })?;
                    state.entities.retain(|e| e.id != entity_dto.id);
                    state.entities.push(entity_dto);
                }
                "ENTITY_DESPAWNED" => {
                    let parsed_id = event
                        .payload_json
                        .get("entity_id")
                        .cloned()
                        .map(serde_json::from_value::<EntityId>)
                        .transpose()
                        .map_err(|e| {
                            PersistenceError::RecoveryFailed(format!(
                                "Failed to parse ENTITY_DESPAWNED entity_id: {e}"
                            ))
                        })?
                        .ok_or_else(|| {
                            PersistenceError::RecoveryFailed(
                                "ENTITY_DESPAWNED payload missing entity_id".into(),
                            )
                        })?;
                    state.entities.retain(|e| e.id != parsed_id);
                }
                "ENTITY_MOVED" => {
                    let parsed_id = event
                        .payload_json
                        .get("entity_id")
                        .cloned()
                        .map(serde_json::from_value::<EntityId>)
                        .transpose()
                        .map_err(|e| {
                            PersistenceError::RecoveryFailed(format!(
                                "Failed to parse ENTITY_MOVED entity_id: {e}"
                            ))
                        })?
                        .ok_or_else(|| {
                            PersistenceError::RecoveryFailed(
                                "ENTITY_MOVED payload missing entity_id".into(),
                            )
                        })?;
                    let new_pos = event
                        .payload_json
                        .get("position")
                        .cloned()
                        .map(serde_json::from_value::<PositionDto>)
                        .transpose()
                        .map_err(|e| {
                            PersistenceError::RecoveryFailed(format!(
                                "Failed to parse ENTITY_MOVED position: {e}"
                            ))
                        })?
                        .ok_or_else(|| {
                            PersistenceError::RecoveryFailed(
                                "ENTITY_MOVED payload missing position".into(),
                            )
                        })?;

                    if let Some(entity) = state.entities.iter_mut().find(|e| e.id == parsed_id) {
                        entity.position = new_pos;
                    }
                }
                "SCHEDULED_ACTION_CREATED" => {
                    let action_dto =
                        serde_json::from_value::<ScheduledActionDto>(event.payload_json.clone())
                            .map_err(|e| {
                                PersistenceError::RecoveryFailed(format!(
                                    "Failed to parse SCHEDULED_ACTION_CREATED payload: {e}"
                                ))
                            })?;
                    scheduled_actions.retain(|a| a.id != action_dto.id);
                    scheduled_actions.push(action_dto);
                }
                "SCHEDULED_ACTION_COMPLETED" | "SCHEDULED_ACTION_CANCELLED" => {
                    let action_id = event
                        .payload_json
                        .get("action_id")
                        .cloned()
                        .map(serde_json::from_value::<Uuid>)
                        .transpose()
                        .map_err(|e| {
                            PersistenceError::RecoveryFailed(format!(
                                "Failed to parse SCHEDULED_ACTION action_id: {e}"
                            ))
                        })?
                        .ok_or_else(|| {
                            PersistenceError::RecoveryFailed(
                                "SCHEDULED_ACTION payload missing action_id".into(),
                            )
                        })?;
                    scheduled_actions.retain(|a| a.id != action_id);
                }
                unknown => {
                    return Err(PersistenceError::RecoveryFailed(format!(
                        "Unknown domain event type '{unknown}' during replay"
                    )));
                }
            }
        }

        state.tick = tick;

        // Verify all events up to the authoritative region sequence were replayed (no missing tail)
        if committed_event_seq != db_seq as u64 {
            return Err(PersistenceError::RecoveryFailed(format!(
                "Event tail missing during recovery: replayed up to sequence {}, but region durable_event_seq is {}",
                committed_event_seq, db_seq
            )));
        }

        // Verify that our lease is still valid and not stolen during recovery
        crate::verify_lease_fencing(pool, region_id, new_epoch).await?;

        // Verify restored invariants
        if state.epoch != new_epoch as u64 {
            return Err(PersistenceError::RecoveryFailed(
                "Restored state epoch must match newly acquired lease epoch".into(),
            ));
        }
        if state.region_id.0 != region_id {
            return Err(PersistenceError::RecoveryFailed(
                "Restored state region ID mismatch".into(),
            ));
        }
        if state.status != game_domain::RegionStatus::Active {
            return Err(PersistenceError::RecoveryFailed(
                "Restored region state must be in ACTIVE status".into(),
            ));
        }

        Ok(RecoveredRegionState {
            region_id,
            epoch: new_epoch,
            tick,
            committed_event_seq,
            rng_seed,
            rng_step,
            state,
            scheduled_actions,
            replayed_events_count,
        })
    }
}
