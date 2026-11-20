use game_domain::{RegionId, RegionStateDto, RegionStatus};
use persistence::{
    apply_migration, RecoveryManager, ScheduledActionDto, SnapshotDto, SnapshotManager,
    CURRENT_ENGINE_VERSION, CURRENT_RULES_VERSION, CURRENT_SCHEMA_VERSION,
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

fn get_test_database_url() -> String {
    if let Ok(url) = env::var("DATABASE_URL") {
        return url;
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
    let database_url = get_test_database_url();

    match PgPool::connect(&database_url).await {
        Ok(pool) => {
            apply_migration(&pool)
                .await
                .expect("Failed to apply schemas/0001_initial.sql");
            Some(pool)
        }
        Err(e) => {
            eprintln!("Skipping PostgreSQL integration test (cannot connect: {e})");
            None
        }
    }
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

    // 1. Save valid Snapshot A at tick 100
    let dto_a = SnapshotDto {
        region_id,
        tick: 100,
        committed_event_seq: 10,
        ownership_epoch: 1,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 777,
        rng_step: 42,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: 1,
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

    // 3. Load latest verified snapshot
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
        "Expected snapshot A to be selected after B failed verification"
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

    // 1. Save base snapshot at tick 50, committed_event_seq 5
    let dto = SnapshotDto {
        region_id,
        tick: 50,
        committed_event_seq: 5,
        ownership_epoch: 1,
        schema_version: CURRENT_SCHEMA_VERSION,
        engine_version: CURRENT_ENGINE_VERSION.to_string(),
        rules_version: CURRENT_RULES_VERSION,
        rng_seed: 12345,
        rng_step: 10,
        state: RegionStateDto {
            region_id: RegionId(region_id),
            epoch: 1,
            tick: 50,
            status: RegionStatus::Active,
            entities: vec![],
        },
        scheduled_actions: vec![],
    };

    SnapshotManager::save_snapshot(&pool, &dto)
        .await
        .expect("save snapshot");

    // 2. Insert domain events 6, 7, 8
    for seq in 6..=8 {
        let event_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO domain_events (
                region_id, region_event_seq, event_id, world_id,
                event_type, schema_version, rules_version, tick,
                correlation_id, payload, created_at
            ) VALUES ($1, $2, $3, $4, 'ENTITY_TICK', 1, 1, $5, $6, $7, clock_timestamp())
            "#,
        )
        .bind(region_id)
        .bind(seq as i64)
        .bind(event_id)
        .bind(world_id)
        .bind(50 + (seq - 5))
        .bind(event_id)
        .bind(serde_json::json!({ "tick": 50 + (seq - 5) }))
        .execute(&pool)
        .await
        .expect("insert domain event");
    }

    // 3. First recovery attempt
    let recovered1 = RecoveryManager::recover_region(&pool, region_id, "worker-node-1", 60)
        .await
        .expect("first recovery attempt");

    assert_eq!(recovered1.tick, 53);
    assert_eq!(recovered1.committed_event_seq, 8);
    assert_eq!(recovered1.replayed_events_count, 3);
    assert_eq!(recovered1.epoch, 1); // First lease acquisition sets epoch = 1

    // 4. Simulate crash right after recovery and run second recovery attempt from new worker
    let recovered2 = RecoveryManager::recover_region(&pool, region_id, "worker-node-2", 60)
        .await
        .expect("second recovery attempt after crash");

    assert_eq!(recovered2.tick, 53);
    assert_eq!(recovered2.committed_event_seq, 8);
    assert_eq!(recovered2.replayed_events_count, 3);
    assert_eq!(recovered2.epoch, 2); // Second recovery advances epoch to 2
    assert_eq!(recovered2.rng_seed, 12345);
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
