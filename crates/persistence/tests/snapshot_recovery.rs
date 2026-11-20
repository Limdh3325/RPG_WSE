use game_domain::{EntityId, EntityStateDto, PositionDto, RegionId, RegionStateDto, RegionStatus};
use persistence::{
    CURRENT_ENGINE_VERSION, CURRENT_RULES_VERSION, CURRENT_SCHEMA_VERSION, RecoveryManager,
    ScheduledActionDto, SnapshotDto, SnapshotManager, apply_migration,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

fn get_test_database_url() -> String {
    if let Ok(url) = env::var("DATABASE_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            if !trimmed.starts_with("postgresql://") && !trimmed.starts_with("postgres://") {
                return format!("postgresql://{trimmed}");
            }
            return trimmed.to_string();
        }
    }
    let proto = "postgresql";
    let user = env::var("DB_USER").unwrap_or_else(|_| "wse_dev".to_string());
    let pass = env::var("DB_PASS").unwrap_or_else(|_| "change-this-local-password".to_string());
    let host = env::var("DB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("DB_PORT").unwrap_or_else(|_| "5433".to_string());
    let db = env::var("DB_NAME").unwrap_or_else(|_| "wse_dev".to_string());
    format!("{proto}://{user}:{pass}@{host}:{port}/{db}")
}

async fn setup_db_pool() -> Option<PgPool> {
    let has_explicit_db = env::var("DATABASE_URL").is_ok();
    let database_url = get_test_database_url();
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            if has_explicit_db {
                panic!("Failed to connect to specified DATABASE_URL ({database_url}): {err}");
            }
            eprintln!("Skipping PostgreSQL integration test (cannot connect: {err})");
            return None;
        }
    };

    if let Err(err) = apply_migration(&pool).await {
        panic!("Failed to apply initial schema migration: {err}");
    }

    Some(pool)
}

async fn create_test_world_and_region(pool: &PgPool) -> (Uuid, Uuid) {
    let world_id = Uuid::new_v4();
    let region_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO worlds (id, name, seed, game_time_ms)
        VALUES ($1, 'Test World', 123456, 0)
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(world_id)
    .execute(pool)
    .await
    .expect("create test world");

    sqlx::query(
        r#"
        INSERT INTO regions (id, world_id, name, grid_width, grid_height)
        VALUES ($1, $2, $3, 100, 100)
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(region_id)
    .bind(world_id)
    .bind(format!("region-{}", &region_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("create test region");

    (world_id, region_id)
}

#[tokio::test]
async fn test_per_004_corrupted_snapshot_falls_back_to_last_valid() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;

    let epoch = persistence::acquire_or_renew_lease(&pool, region_id, "test-worker", 60)
        .await
        .expect("acquire lease");

    // 1. Save valid Snapshot A at tick 100
    let dto_a = SnapshotDto {
        region_id,
        tick: 100,
        committed_event_seq: 0,
        ownership_epoch: epoch,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 777,
        rng_step: 42,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: epoch as u64,
            tick: 100,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![ScheduledActionDto {
            id: Uuid::new_v4(),
            entity_id: None,
            due_tick: 150,
            action_type: "SPAWN_WAVE".to_string(),
            payload: serde_json::json!({ "wave": 2 }),
        }],
    };

    let id_a = SnapshotManager::save_snapshot(&pool, &dto_a)
        .await
        .expect("save snapshot A");

    // 2. Insert corrupted Snapshot B at tick 200 (corrupted payload simulating crash during write)
    let id_b = Uuid::new_v4();
    let fake_sha256 = vec![0u8; 32];
    let corrupt_payload = b"{\"corrupt\": true, incomplete...".to_vec();

    sqlx::query(
        r#"
        INSERT INTO region_snapshots (
            id, region_id, tick, committed_event_seq, ownership_epoch,
            schema_version, engine_version, rules_version, sha256, payload
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(id_b)
    .bind(region_id)
    .bind(200i64)
    .bind(25i64)
    .bind(1i64)
    .bind(CURRENT_SCHEMA_VERSION as i32)
    .bind(CURRENT_ENGINE_VERSION)
    .bind(CURRENT_RULES_VERSION as i32)
    .bind(&fake_sha256)
    .bind(&corrupt_payload)
    .execute(&pool)
    .await
    .expect("insert corrupt snapshot B");

    // 3. Insert Incompatible Version Snapshot C at tick 300 (valid payload but incompatible schema version = 999)
    let id_c = Uuid::new_v4();
    let mut dto_c = dto_a.clone();
    dto_c.tick = 300;
    dto_c.schema_version = 999;
    let payload_c = serde_json::to_vec(&dto_c).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&payload_c);
    let hash_c = hasher.finalize().to_vec();

    sqlx::query(
        r#"
        INSERT INTO region_snapshots (
            id, region_id, tick, committed_event_seq, ownership_epoch,
            schema_version, engine_version, rules_version, sha256, payload
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(id_c)
    .bind(region_id)
    .bind(300i64)
    .bind(30i64)
    .bind(1i64)
    .bind(999i32) // Incompatible schema version
    .bind(CURRENT_ENGINE_VERSION)
    .bind(CURRENT_RULES_VERSION as i32)
    .bind(&hash_c)
    .bind(&payload_c)
    .execute(&pool)
    .await
    .expect("insert incompatible snapshot C");

    // 4. Load latest verified snapshot:
    // Snapshot C is rejected due to incompatible version;
    // Snapshot B is rejected due to corrupted SHA256;
    // Must fall back to Snapshot A!
    let loaded = SnapshotManager::load_latest_verified_snapshot(&pool, region_id)
        .await
        .expect("load verified snapshot");

    assert!(
        loaded.is_some(),
        "Expected fallback to valid snapshot A, but got None"
    );

    let (loaded_id, loaded_dto) = loaded.unwrap();
    assert_eq!(
        loaded_id, id_a,
        "Expected snapshot A to be selected after B (corrupted) and C (incompatible) failed verification"
    );
    assert_eq!(loaded_dto.tick, 100);
    assert_eq!(loaded_dto.rng_seed, 777);
    assert_eq!(loaded_dto.scheduled_actions.len(), 1);
    assert_eq!(loaded_dto.scheduled_actions[0].action_type, "SPAWN_WAVE");
}

#[tokio::test]
async fn test_per_005_recovery_replays_events_and_is_crash_idempotent() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (world_id, region_id) = create_test_world_and_region(&pool).await;

    let epoch = persistence::acquire_or_renew_lease(&pool, region_id, "test-worker", 60)
        .await
        .expect("acquire lease");

    sqlx::query("UPDATE regions SET durable_event_seq = 5 WHERE id = $1")
        .bind(region_id)
        .execute(&pool)
        .await
        .expect("update durable_event_seq");

    // 1. Save base snapshot at tick 50, committed_event_seq 5
    let dto = SnapshotDto {
        region_id,
        tick: 50,
        committed_event_seq: 5,
        ownership_epoch: epoch,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 12345,
        rng_step: 10,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: epoch as u64,
            tick: 50,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![],
    };

    SnapshotManager::save_snapshot(&pool, &dto)
        .await
        .expect("save snapshot");

    let test_entity_id = EntityId(Uuid::new_v4());
    let initial_pos = PositionDto {
        x_millitile: 10000,
        y_millitile: 20000,
    };
    let moved_pos = PositionDto {
        x_millitile: 35000,
        y_millitile: 45000,
    };
    let action_id = Uuid::new_v4();

    // 2. Insert domain events 6, 7, 8, 9
    // Event 6: ENTITY_TICK
    let ev6 = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO domain_events (
            region_id, region_event_seq, event_id, world_id,
            event_type, schema_version, rules_version, tick,
            correlation_id, payload, created_at
        ) VALUES ($1, 6, $2, $3, 'ENTITY_TICK', 1, 1, 51, $2, $4, clock_timestamp())
        "#,
    )
    .bind(region_id)
    .bind(ev6)
    .bind(world_id)
    .bind(serde_json::json!({ "tick": 51 }))
    .execute(&pool)
    .await
    .expect("insert ev6");

    // Event 7: ENTITY_SPAWNED
    let ev7 = Uuid::new_v4();
    let spawned_entity_dto = EntityStateDto {
        id: test_entity_id,
        position: initial_pos,
        velocity: None,
        health: None,
        needs: None,
        personality: None,
        inventory: None,
        faction: None,
        collider: None,
    };
    sqlx::query(
        r#"
        INSERT INTO domain_events (
            region_id, region_event_seq, event_id, world_id,
            event_type, schema_version, rules_version, tick,
            correlation_id, payload, created_at
        ) VALUES ($1, 7, $2, $3, 'ENTITY_SPAWNED', 1, 1, 52, $2, $4, clock_timestamp())
        "#,
    )
    .bind(region_id)
    .bind(ev7)
    .bind(world_id)
    .bind(serde_json::to_value(&spawned_entity_dto).unwrap())
    .execute(&pool)
    .await
    .expect("insert ev7");

    // Event 8: ENTITY_MOVED
    let ev8 = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO domain_events (
            region_id, region_event_seq, event_id, world_id,
            event_type, schema_version, rules_version, tick,
            correlation_id, payload, created_at
        ) VALUES ($1, 8, $2, $3, 'ENTITY_MOVED', 1, 1, 53, $2, $4, clock_timestamp())
        "#,
    )
    .bind(region_id)
    .bind(ev8)
    .bind(world_id)
    .bind(serde_json::json!({
        "tick": 53,
        "entity_id": test_entity_id,
        "position": moved_pos,
    }))
    .execute(&pool)
    .await
    .expect("insert ev8");

    // Event 9: SCHEDULED_ACTION_CREATED
    let ev9 = Uuid::new_v4();
    let scheduled_action = ScheduledActionDto {
        id: action_id,
        entity_id: None,
        due_tick: 80,
        action_type: "SPAWN_AMBUSH".to_string(),
        payload: serde_json::json!({ "monsters": 3 }),
    };
    sqlx::query(
        r#"
        INSERT INTO domain_events (
            region_id, region_event_seq, event_id, world_id,
            event_type, schema_version, rules_version, tick,
            correlation_id, payload, created_at
        ) VALUES ($1, 9, $2, $3, 'SCHEDULED_ACTION_CREATED', 1, 1, 54, $2, $4, clock_timestamp())
        "#,
    )
    .bind(region_id)
    .bind(ev9)
    .bind(world_id)
    .bind(serde_json::to_value(&scheduled_action).unwrap())
    .execute(&pool)
    .await
    .expect("insert ev9");

    sqlx::query("UPDATE regions SET durable_event_seq = 9 WHERE id = $1")
        .bind(region_id)
        .execute(&pool)
        .await
        .expect("update region durable_event_seq");

    // 3. First recovery attempt
    let recovered1 = RecoveryManager::recover_region(&pool, region_id, "worker-node-1", 60)
        .await
        .expect("first recovery attempt");

    assert_eq!(recovered1.tick, 54);
    assert_eq!(recovered1.committed_event_seq, 9);
    assert_eq!(recovered1.replayed_events_count, 4);
    assert_eq!(recovered1.epoch, epoch + 1);
    assert_eq!(recovered1.rng_step, 11); // advanced by ENTITY_TICK

    // Verify state mutations applied by event reducer
    assert_eq!(recovered1.state.entities.len(), 1);
    assert_eq!(recovered1.state.entities[0].id, test_entity_id);
    assert_eq!(recovered1.state.entities[0].position, moved_pos);
    assert_eq!(recovered1.scheduled_actions.len(), 1);
    assert_eq!(recovered1.scheduled_actions[0].id, action_id);
    assert_eq!(recovered1.scheduled_actions[0].action_type, "SPAWN_AMBUSH");

    // 4. Simulate crash right after recovery and run second recovery attempt from new worker
    let recovered2 = RecoveryManager::recover_region(&pool, region_id, "worker-node-2", 60)
        .await
        .expect("second recovery attempt after crash");

    assert_eq!(recovered2.tick, 54);
    assert_eq!(recovered2.committed_event_seq, 9);
    assert_eq!(recovered2.replayed_events_count, 4);
    assert_eq!(recovered2.epoch, epoch + 2); // Second recovery advances epoch further
    assert_eq!(recovered2.state.entities.len(), 1);
    assert_eq!(recovered2.state.entities[0].position, moved_pos);
    assert_eq!(recovered2.scheduled_actions.len(), 1);
}

#[tokio::test]
async fn test_recovery_on_fresh_region_without_snapshots() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;

    let recovered = RecoveryManager::recover_region(&pool, region_id, "worker-node-fresh", 60)
        .await
        .expect("fresh recovery without snapshot");

    assert_eq!(recovered.tick, 0);
    assert_eq!(recovered.committed_event_seq, 0);
    assert_eq!(recovered.replayed_events_count, 0);
    assert_eq!(recovered.epoch, 1);
    assert_eq!(recovered.state.status, RegionStatus::Active);
}

#[tokio::test]
async fn test_per_011_fenced_checkpoint_and_invariants_verification() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;

    // 1. Acquire lease to get valid epoch
    let epoch = persistence::acquire_or_renew_lease(&pool, region_id, "checkpoint-worker", 60)
        .await
        .expect("acquire lease");

    let dto = SnapshotDto {
        region_id,
        tick: 100,
        committed_event_seq: 0,
        ownership_epoch: epoch,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 555,
        rng_step: 0,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: epoch as u64,
            tick: 100,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![],
    };

    // 2. Fenced checkpoint with matching epoch succeeds
    let snap_id = SnapshotManager::save_snapshot_fenced(&pool, &dto, epoch)
        .await
        .expect("fenced snapshot save must succeed with valid epoch");
    assert!(!snap_id.is_nil());

    // 3. Fenced checkpoint with stale epoch is rejected
    let stale_res = SnapshotManager::save_snapshot_fenced(&pool, &dto, epoch - 1).await;
    assert!(
        matches!(
            stale_res,
            Err(persistence::PersistenceError::LeaseFencingFailed { .. })
        ),
        "Checkpoint with stale epoch must be rejected"
    );

    // 4. Checkpoint watermark exceeding database durable_event_seq is rejected
    let mut invalid_watermark_dto = dto.clone();
    invalid_watermark_dto.committed_event_seq = 99999;
    let watermark_res =
        SnapshotManager::save_snapshot_fenced(&pool, &invalid_watermark_dto, epoch).await;
    assert!(
        matches!(
            watermark_res,
            Err(persistence::PersistenceError::InvalidOperation(_))
        ),
        "Checkpoint watermark exceeding database sequence must be rejected"
    );

    // 5. Recovery pipeline restores state and validates invariants
    let recovered = RecoveryManager::recover_region(&pool, region_id, "recovery-worker", 60)
        .await
        .expect("recovery must succeed");

    assert_eq!(recovered.region_id, region_id);
    assert_eq!(recovered.epoch, epoch + 1); // Incremented for new worker takeover
    assert_eq!(recovered.tick, 100);
    assert_eq!(recovered.state.status, RegionStatus::Active);
    assert_eq!(recovered.state.epoch, (epoch + 1) as u64);
}

#[tokio::test]
async fn test_per_014_snapshot_version_and_fencing_guards() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;
    let epoch = persistence::acquire_or_renew_lease(&pool, region_id, "worker-guard", 60)
        .await
        .expect("acquire lease");

    let dto = SnapshotDto {
        region_id,
        tick: 1,
        committed_event_seq: 0,
        ownership_epoch: epoch,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 42,
        rng_step: 0,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: epoch as u64,
            tick: 1,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![],
    };

    // 1. Version incompatibility rejected
    let mut incompatible_dto = dto.clone();
    incompatible_dto.rules_version = CURRENT_RULES_VERSION + 1;
    let err_incomp = SnapshotManager::save_snapshot_fenced(&pool, &incompatible_dto, epoch).await;
    assert!(
        matches!(
            err_incomp,
            Err(persistence::PersistenceError::IncompatibleVersion { .. })
        ),
        "Incompatible rules version must be rejected"
    );

    // 2. Unfenced delegate check & stale payload epoch check
    let new_epoch = persistence::acquire_or_renew_lease(&pool, region_id, "worker-guard-2", 60)
        .await
        .expect("acquire new lease");
    assert!(new_epoch > epoch);

    // Attempt save with old epoch DTO via public save_snapshot (delegates to fencing with dto.ownership_epoch)
    let stale_public = SnapshotManager::save_snapshot(&pool, &dto).await;
    assert!(
        matches!(
            stale_public,
            Err(persistence::PersistenceError::LeaseFencingFailed { .. })
        ),
        "Public save_snapshot with stale epoch must be rejected by lease fencing"
    );

    // Attempt save with mismatched expected_epoch vs dto.ownership_epoch
    let stale_payload = SnapshotManager::save_snapshot_fenced(&pool, &dto, new_epoch).await;
    assert!(
        matches!(
            stale_payload,
            Err(persistence::PersistenceError::LeaseFencingFailed { .. })
        ),
        "Fenced save with payload epoch differing from current fencing token must be rejected"
    );
}

#[tokio::test]
async fn test_per_016_missing_event_tail_fails_recovery() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;

    // Simulate region has durable_event_seq = 5, but domain_events table is completely empty (tail missing)
    sqlx::query("UPDATE regions SET durable_event_seq = 5 WHERE id = $1")
        .bind(region_id)
        .execute(&pool)
        .await
        .unwrap();

    let result = RecoveryManager::recover_region(&pool, region_id, "worker-missing-tail", 60).await;
    assert!(
        matches!(result, Err(persistence::PersistenceError::RecoveryFailed(ref msg)) if msg.contains("Event tail missing")),
        "Recovery must fail when events up to durable_event_seq are missing"
    );
}

#[tokio::test]
async fn test_per_018_only_incompatible_snapshot_rejects_recovery_without_fallback() {
    let pool = match setup_db_pool().await {
        Some(p) => p,
        None => return,
    };
    let (_world_id, region_id) = create_test_world_and_region(&pool).await;

    // Insert only an incompatible snapshot (rules_version = 999) with valid SHA-256 hash
    let snap_id = Uuid::new_v4();
    let dto = SnapshotDto {
        region_id,
        tick: 250,
        committed_event_seq: 0,
        ownership_epoch: 1,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: 999, // Incompatible
        rng_seed: 42,
        rng_step: 42,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: 1,
            tick: 250,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![],
    };
    let payload = serde_json::to_vec(&dto).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let sha256_hash = hasher.finalize().to_vec();

    sqlx::query(
        r#"
        INSERT INTO region_snapshots (
            id, region_id, tick, committed_event_seq, ownership_epoch,
            schema_version, engine_version, rules_version, sha256, payload
        ) VALUES ($1, $2, 250, 0, 1, 1, $3, 999, $4, $5)
        "#,
    )
    .bind(snap_id)
    .bind(region_id)
    .bind(CURRENT_ENGINE_VERSION)
    .bind(&sha256_hash)
    .bind(&payload)
    .execute(&pool)
    .await
    .unwrap();

    // With no valid earlier snapshot, recovery must fail with IncompatibleVersion instead of silently resetting to genesis (tick 0)
    let res =
        RecoveryManager::recover_region(&pool, region_id, "worker-incompatible-check", 60).await;
    assert!(
        matches!(
            res,
            Err(persistence::PersistenceError::IncompatibleVersion { .. })
        ),
        "Recovery must fail with IncompatibleVersion when only incompatible snapshot exists"
    );
}
