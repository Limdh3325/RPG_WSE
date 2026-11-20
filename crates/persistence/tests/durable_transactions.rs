use persistence::{
    CommandReceiptRecord, DurableEvent, DurableIntent, DurableOperation, DurableWriter,
    EventDeduplicator, OutboxPublisher, PersistenceError, ReceiptStatus, SqlxEventDeduplicator,
    acquire_or_renew_lease, apply_migration,
};
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

async fn setup_test_pool() -> Option<PgPool> {
    let has_explicit_db = env::var("DATABASE_URL").is_ok();
    let url = get_test_database_url();
    let pool = match PgPool::connect(&url).await {
        Ok(p) => p,
        Err(err) => {
            if has_explicit_db {
                panic!("Failed to connect to specified DATABASE_URL: {err}");
            }
            eprintln!("Skipping PostgreSQL integration tests (cannot connect: {err})");
            return None;
        }
    };

    if let Err(err) = apply_migration(&pool).await {
        panic!("Failed to apply initial schema migration: {err}");
    }

    Some(pool)
}

struct TestFixture {
    pub world_id: Uuid,
    pub region_id: Uuid,
    pub actor_id: Uuid,
    pub buyer_wallet_id: Uuid,
    pub settlement_id: Uuid,
    pub epoch: i64,
}

async fn create_test_fixture(
    pool: &PgPool,
    initial_balance: i64,
    initial_stock: i64,
) -> TestFixture {
    let world_id = Uuid::new_v4();
    let region_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let settlement_id = region_id;

    // 1. World
    sqlx::query(
        "INSERT INTO worlds (id, name, seed, rules_version) VALUES ($1, $2, 42, 1) ON CONFLICT (id) DO NOTHING"
    )
    .bind(world_id)
    .bind("Test World")
    .execute(pool)
    .await
    .expect("insert world");

    // 2. Region
    sqlx::query(
        "INSERT INTO regions (id, world_id, name, grid_width, grid_height, durable_event_seq) VALUES ($1, $2, $3, 100, 100, 0) ON CONFLICT (id) DO NOTHING"
    )
    .bind(region_id)
    .bind(world_id)
    .bind(format!("region-{}", &region_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("insert region");

    // 3. Region Lease
    let epoch = acquire_or_renew_lease(pool, region_id, "test_worker_1", 60)
        .await
        .expect("acquire lease");

    // 4. Player Entity & Wallet
    sqlx::query(
        "INSERT INTO entities (id, region_id, kind, state_version, persistent_json) VALUES ($1, $2, 'PLAYER', 1, '{}') ON CONFLICT (id) DO NOTHING"
    )
    .bind(actor_id)
    .bind(region_id)
    .execute(pool)
    .await
    .expect("insert entity");

    sqlx::query(
        "INSERT INTO wallets (entity_id, gold, version) VALUES ($1, $2, 1) ON CONFLICT (entity_id) DO UPDATE SET gold = $2, version = 1"
    )
    .bind(actor_id)
    .bind(initial_balance)
    .execute(pool)
    .await
    .expect("insert wallet");

    // 5. Settlement
    sqlx::query(
        "INSERT INTO settlements (id, region_id, name, population, food_stock, food_capacity, security_index, shortage_active, version) VALUES ($1, $2, $3, 100, 50, 100, 500, false, 1) ON CONFLICT (id) DO NOTHING"
    )
    .bind(settlement_id)
    .bind(region_id)
    .bind(format!("town-{}", &settlement_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("insert settlement");

    // 6. Item Catalog
    sqlx::query(
        "INSERT INTO item_catalog (sku, name, stackable, max_stack, base_price) VALUES ($1, 'Healing Potion', true, 99, 10) ON CONFLICT (sku) DO NOTHING"
    )
    .bind("potion_healing")
    .execute(pool)
    .await
    .expect("insert item catalog");

    // 7. Market Stock
    sqlx::query(
        "INSERT INTO market_stock (settlement_id, sku, quantity, price, target_stock, version) VALUES ($1, $2, $3, 10, 100, 1) ON CONFLICT (settlement_id, sku) DO UPDATE SET quantity = $3, price = 10, version = 1"
    )
    .bind(settlement_id)
    .bind("potion_healing")
    .bind(initial_stock)
    .execute(pool)
    .await
    .expect("insert stock");

    TestFixture {
        world_id,
        region_id,
        actor_id,
        buyer_wallet_id: actor_id,
        settlement_id,
        epoch,
    }
}

async fn get_wallet_balance(pool: &PgPool, entity_id: Uuid) -> i64 {
    let (gold,): (i64,) = sqlx::query_as("SELECT gold FROM wallets WHERE entity_id = $1")
        .bind(entity_id)
        .fetch_one(pool)
        .await
        .expect("fetch wallet");
    gold
}

async fn get_market_stock(pool: &PgPool, settlement_id: Uuid, sku: &str) -> i64 {
    let (qty,): (i64,) =
        sqlx::query_as("SELECT quantity FROM market_stock WHERE settlement_id = $1 AND sku = $2")
            .bind(settlement_id)
            .bind(sku)
            .fetch_one(pool)
            .await
            .expect("fetch stock");
    qty
}

#[tokio::test]
async fn test_per_001_failpoint_before_begin_no_state_change() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());
    writer.set_failpoint("before_begin").await;

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [1u8; 32],
        tick: 10,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5,
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    let result = writer.commit_intent(intent).await;
    assert!(
        result.is_err(),
        "Expected failure due to before_begin failpoint"
    );

    // Asset state untouched
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );

    // No receipt recorded
    let receipt = writer
        .query_receipt(command_id)
        .await
        .expect("query receipt");
    assert!(receipt.is_none());
}

#[tokio::test]
async fn test_per_002_failpoint_before_commit_rolls_back() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());
    writer.set_failpoint("before_commit").await;

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [2u8; 32],
        tick: 11,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5,
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    let result = writer.commit_intent(intent).await;
    assert!(result.is_err(), "Expected rollback failure");

    // Atomic rollback: balance and stock unchanged!
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );

    // Receipt is rolled back as well
    let receipt = writer
        .query_receipt(command_id)
        .await
        .expect("query receipt");
    assert!(receipt.is_none(), "Pending receipt should be rolled back");
}

#[tokio::test]
async fn test_per_003_commit_succeeds_ack_lost_lookup_verifies_success() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());
    writer.set_failpoint("after_commit_before_ack").await;

    let command_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [3u8; 32],
        tick: 12,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5,
            unit_price: 10,
        }],
        emitted_events: vec![DurableEvent {
            event_id,
            event_type: "ItemPurchased".to_string(),
            schema_version: 1,
            rules_version: 1,
            tick: 12,
            causation_event_id: None,
            correlation_id: command_id,
            payload: serde_json::json!({ "sku": "potion_healing", "quantity": 5 }),
            outbox_topic: Some("domain.events.trade".to_string()),
        }],
    };

    // Commit with simulated ACK loss
    let result = writer.commit_intent(intent).await;
    assert!(result.is_err(), "Simulated network drop before ACK return");

    // Verification via query_receipt: Transaction actually committed in DB!
    let receipt = writer
        .query_receipt(command_id)
        .await
        .expect("query receipt");
    let record: CommandReceiptRecord = receipt.expect("Receipt must exist after commit");
    assert_eq!(record.status, ReceiptStatus::Succeeded);

    // Assets deducted exactly once
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        950
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        45
    );

    // Outbox entry exists
    let (outbox_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM outbox WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .expect("count outbox");
    assert_eq!(outbox_count, 1);
}

#[tokio::test]
async fn test_per_004_lease_fencing_rejects_stale_epoch() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    let stale_epoch = fixture.epoch - 1; // Stale epoch
    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: stale_epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [4u8; 32],
        tick: 20,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: -100,
        }],
        emitted_events: vec![],
    };

    let result = writer.commit_intent(intent).await;
    match result {
        Err(PersistenceError::LeaseFencingFailed {
            region_id,
            expected_epoch,
            actual_epoch,
        }) => {
            assert_eq!(region_id, fixture.region_id);
            assert_eq!(expected_epoch, stale_epoch);
            assert_eq!(actual_epoch, Some(fixture.epoch));
        }
        other => panic!("Expected LeaseFencingFailed, got: {other:?}"),
    }

    // Wallet balance untouched
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
}

#[tokio::test]
async fn test_per_005_idempotency_100_replays_exactly_one_charge() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    let command_id = Uuid::new_v4();
    let payload_hash = [42u8; 32];
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash,
        tick: 30,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 3,
            unit_price: 10, // 30 gold total
        }],
        emitted_events: vec![],
    };

    // First call: succeeds and commits
    let first_ack = writer
        .commit_intent(intent.clone())
        .await
        .expect("First commit should succeed");
    assert_eq!(first_ack.status, ReceiptStatus::Succeeded);
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        970
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        47
    );

    // Replay 99 times with the exact same command_id and payload
    for _ in 1..100 {
        let ack = writer
            .commit_intent(intent.clone())
            .await
            .expect("Replayed request should return cached ACK");
        assert_eq!(ack.status, ReceiptStatus::Succeeded);
    }

    // Verify: Exactly 1 payment deducted, 99 replays caused ZERO additional deductions!
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        970,
        "Balance should only be deducted once despite 100 attempts"
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        47,
        "Stock should only be deducted once despite 100 attempts"
    );
}

#[tokio::test]
async fn test_per_006_different_payload_reuse_rejected() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    let command_id = Uuid::new_v4();
    let mut intent1 = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [5u8; 32],
        tick: 35,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: -50,
        }],
        emitted_events: vec![],
    };

    writer
        .commit_intent(intent1.clone())
        .await
        .expect("Original commit");

    // Reuse command_id with a different payload_hash
    intent1.payload_hash = [6u8; 32];
    let res = writer.commit_intent(intent1).await;
    match res {
        Err(PersistenceError::CommandHashMismatch { command_id: cid }) => {
            assert_eq!(cid, command_id);
        }
        other => panic!("Expected CommandHashMismatch, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_per_007_two_buyers_competing_for_last_stock() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    // Initialize stock = 1!
    let fixture = create_test_fixture(&pool, 1000, 1).await;

    // Create a second buyer entity and wallet
    let buyer2_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO entities (id, region_id, kind, state_version, persistent_json) VALUES ($1, $2, 'PLAYER', 1, '{}')"
    )
    .bind(buyer2_id)
    .bind(fixture.region_id)
    .execute(&pool)
    .await
    .expect("insert buyer 2");

    sqlx::query("INSERT INTO wallets (entity_id, gold, version) VALUES ($1, 1000, 1)")
        .bind(buyer2_id)
        .execute(&pool)
        .await
        .expect("insert wallet 2");

    let writer1 = DurableWriter::new(pool.clone());
    let writer2 = DurableWriter::new(pool.clone());

    let intent1 = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [7u8; 32],
        tick: 50,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 1,
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    let intent2 = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(buyer2_id),
        payload_hash: [8u8; 32],
        tick: 50,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: buyer2_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 1,
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    // Run both purchases concurrently
    let (res1, res2) = tokio::join!(
        writer1.commit_intent(intent1),
        writer2.commit_intent(intent2)
    );

    // Exactly one must succeed, one must fail with InsufficientStock!
    let successes = (res1.is_ok() as usize) + (res2.is_ok() as usize);
    assert_eq!(
        successes, 1,
        "Exactly one of two concurrent buyers must succeed"
    );

    // Stock must be exactly 0!
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        0
    );

    // Total gold deducted across both buyers must be exactly 10!
    let b1_bal = get_wallet_balance(&pool, fixture.buyer_wallet_id).await;
    let b2_bal = get_wallet_balance(&pool, buyer2_id).await;
    assert_eq!(
        b1_bal + b2_bal,
        1990,
        "Total gold deducted must be exactly 10"
    );
}

#[tokio::test]
async fn test_per_008_outbox_relay_and_consumer_deduplication() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    let command_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [9u8; 32],
        tick: 60,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: -20,
        }],
        emitted_events: vec![DurableEvent {
            event_id,
            event_type: "GoldDeducted".to_string(),
            schema_version: 1,
            rules_version: 1,
            tick: 60,
            causation_event_id: None,
            correlation_id: command_id,
            payload: serde_json::json!({ "delta": -20 }),
            outbox_topic: Some("events.wallet".to_string()),
        }],
    };

    writer.commit_intent(intent).await.expect("commit intent");

    // Test OutboxPublisher
    let publisher = OutboxPublisher::new(pool.clone());
    let deduplicator = EventDeduplicator::new();

    let dedup = deduplicator.clone();
    let published_count = publisher
        .publish_pending(10, |record| {
            let dedup = dedup.clone();
            async move {
                // Simulate consumer processing with deduplicator
                let is_new = dedup.process_if_new(record.event_id).await;
                assert!(is_new, "First receipt must be new");
                Ok(())
            }
        })
        .await
        .expect("publish pending");

    assert!(published_count >= 1);

    // Test SqlxEventDeduplicator (durable across restarts)
    let sqlx_dedup = SqlxEventDeduplicator::new(pool.clone(), "consumer-service-1");
    let is_new = sqlx_dedup
        .process_if_new(event_id)
        .await
        .expect("claim new event");
    assert!(
        is_new,
        "First receipt must be new in durable consumer inbox"
    );

    // Simulate duplicate delivery to consumer
    let is_dup = sqlx_dedup.is_duplicate(event_id).await.expect("check dup");
    assert!(is_dup, "Event must be identified as duplicate in DB");

    // Simulate worker process restart with new deduplicator instance
    let restarted_dedup = SqlxEventDeduplicator::new(pool.clone(), "consumer-service-1");
    let is_new_after_restart = restarted_dedup
        .process_if_new(event_id)
        .await
        .expect("claim after restart");
    assert!(
        !is_new_after_restart,
        "Durable consumer inbox must reject duplicate even after process restart"
    );

    // Also verify transient deduplicator
    let is_dup_mem = deduplicator.is_duplicate(event_id).await;
    assert!(
        is_dup_mem,
        "Event must be identified as duplicate in memory"
    );
}

#[tokio::test]
async fn test_per_009_admission_circuit_breaker() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    // Pause admission (simulating DB circuit breaker tripped)
    writer.set_admitting(false);

    let intent = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [10u8; 32],
        tick: 70,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: -10,
        }],
        emitted_events: vec![],
    };

    let result = writer.commit_intent(intent.clone()).await;
    match result {
        Err(PersistenceError::AdmissionPaused) => {}
        other => panic!("Expected AdmissionPaused, got {other:?}"),
    }

    // Resume admission
    writer.set_admitting(true);
    let ok = writer.commit_intent(intent).await;
    assert!(ok.is_ok(), "Commit must succeed when admission is resumed");
}

#[tokio::test]
async fn test_per_010_negative_or_overflow_purchase_rejected() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    // 1. Negative quantity must be rejected and leave all balances/stocks untouched
    let intent_negative_qty = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [11u8; 32],
        tick: 80,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: -1,
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    let res1 = writer.commit_intent(intent_negative_qty).await;
    match res1 {
        Err(PersistenceError::InvalidOperation(msg)) => {
            assert!(msg.contains("strictly positive"));
        }
        other => panic!("Expected InvalidOperation for negative quantity, got {other:?}"),
    }

    // 2. Negative unit_price must be rejected
    let intent_negative_price = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [12u8; 32],
        tick: 81,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5,
            unit_price: -10,
        }],
        emitted_events: vec![],
    };

    let res2 = writer.commit_intent(intent_negative_price).await;
    assert!(matches!(res2, Err(PersistenceError::InvalidOperation(_))));

    // 3. Overflow total cost must be rejected
    let intent_overflow = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [13u8; 32],
        tick: 82,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: i64::MAX,
            unit_price: 2,
        }],
        emitted_events: vec![],
    };

    let res3 = writer.commit_intent(intent_overflow).await;
    assert!(matches!(res3, Err(PersistenceError::InvalidOperation(_))));

    // Assert asset state is 100% untouched
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );
}

#[tokio::test]
async fn test_per_012_concurrent_apply_migration_no_leak_and_exact_one_record() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };

    // Run 10 concurrent migration calls directly on the pool via tokio::join!
    let (r1, r2, r3, r4, r5, r6, r7, r8, r9, r10) = tokio::join!(
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
        apply_migration(&pool),
    );

    for res in [r1, r2, r3, r4, r5, r6, r7, r8, r9, r10] {
        assert!(res.is_ok(), "Concurrent migration must succeed: {res:?}");
    }

    // Verify exactly one '0001_initial' record in _schema_migrations
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM _schema_migrations WHERE version = $1")
            .bind("0001_initial")
            .fetch_one(&pool)
            .await
            .expect("count migration records");
    assert_eq!(count, 1);

    // Verify zero lingering advisory locks
    let (advisory_locks,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM pg_locks WHERE locktype = 'advisory'")
            .fetch_one(&pool)
            .await
            .expect("count advisory locks");
    assert_eq!(
        advisory_locks, 0,
        "All transaction-scoped advisory locks must be released"
    );
}

#[tokio::test]
async fn test_per_013_i64_min_transfer_safely_rejected_without_panic() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 100, 10).await;
    let writer = DurableWriter::new(pool.clone());

    // 1. Wallet transfer with i64::MIN must not panic with arithmetic negation overflow
    let intent_wallet_min = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [91u8; 32],
        tick: 40,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: i64::MIN,
        }],
        emitted_events: vec![],
    };
    let res_wallet = writer.commit_intent(intent_wallet_min).await;
    assert!(
        matches!(
            res_wallet,
            Err(PersistenceError::InsufficientFunds { .. })
                | Err(PersistenceError::InvalidOperation(_))
        ),
        "Wallet transfer of i64::MIN must return error, not panic"
    );

    // 2. Inventory transfer with i64::MIN must not panic
    let intent_inv_min = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [92u8; 32],
        tick: 41,
        operations: vec![DurableOperation::InventoryTransfer {
            owner_entity_id: fixture.actor_id,
            sku: "potion_healing".into(),
            quantity_delta: i64::MIN,
        }],
        emitted_events: vec![],
    };
    let res_inv = writer.commit_intent(intent_inv_min).await;
    assert!(
        matches!(
            res_inv,
            Err(PersistenceError::InsufficientStock { .. })
                | Err(PersistenceError::InvalidOperation(_))
        ),
        "Inventory transfer of i64::MIN must return error, not panic"
    );

    // Verify balances untouched
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        100
    );
}

#[tokio::test]
async fn test_per_015_inventory_deduction_and_inbox_tx() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    // 1. Initial inventory deposit: +10 potions
    let intent_add = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [93u8; 32],
        tick: 42,
        operations: vec![DurableOperation::InventoryTransfer {
            owner_entity_id: fixture.actor_id,
            sku: "potion_healing".into(),
            quantity_delta: 10,
        }],
        emitted_events: vec![],
    };
    let ack_add = writer
        .commit_intent(intent_add)
        .await
        .expect("add 10 items");
    assert_eq!(ack_add.status, ReceiptStatus::Succeeded);

    let (qty1,): (i64,) = sqlx::query_as(
        "SELECT quantity FROM inventory_slots WHERE owner_entity_id = $1 AND sku = $2",
    )
    .bind(fixture.actor_id)
    .bind("potion_healing")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qty1, 10);

    // 2. Normal deduction: -3 potions (CHECK (quantity >= 0) must NOT fail on INSERT/UPDATE)
    let intent_sub = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [94u8; 32],
        tick: 43,
        operations: vec![DurableOperation::InventoryTransfer {
            owner_entity_id: fixture.actor_id,
            sku: "potion_healing".into(),
            quantity_delta: -3,
        }],
        emitted_events: vec![],
    };
    let ack_sub = writer.commit_intent(intent_sub).await.expect("sub 3 items");
    assert_eq!(ack_sub.status, ReceiptStatus::Succeeded);

    let (qty2,): (i64,) = sqlx::query_as(
        "SELECT quantity FROM inventory_slots WHERE owner_entity_id = $1 AND sku = $2",
    )
    .bind(fixture.actor_id)
    .bind("potion_healing")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qty2, 7);

    // 3. Over-deduction: -8 potions (must return InsufficientStock and keep 7)
    let intent_over = DurableIntent {
        command_id: Uuid::new_v4(),
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [95u8; 32],
        tick: 44,
        operations: vec![DurableOperation::InventoryTransfer {
            owner_entity_id: fixture.actor_id,
            sku: "potion_healing".into(),
            quantity_delta: -8,
        }],
        emitted_events: vec![],
    };
    let err_over = writer.commit_intent(intent_over).await;
    assert!(matches!(
        err_over,
        Err(PersistenceError::InsufficientStock {
            available: 7,
            required: 8,
            ..
        })
    ));

    let (qty3,): (i64,) = sqlx::query_as(
        "SELECT quantity FROM inventory_slots WHERE owner_entity_id = $1 AND sku = $2",
    )
    .bind(fixture.actor_id)
    .bind("potion_healing")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qty3, 7);

    // 4. Test transactional event deduplicator
    let dedup = SqlxEventDeduplicator::new(pool.clone(), "consumer-test");
    let test_ev = Uuid::new_v4();

    // Rollback test: should not record in consumer_inbox
    let mut tx = pool.begin().await.unwrap();
    let is_new = dedup.process_if_new_tx(&mut tx, test_ev).await.unwrap();
    assert!(is_new);
    tx.rollback().await.unwrap();
    assert!(!dedup.is_duplicate(test_ev).await.unwrap());

    // Commit test: should record in consumer_inbox
    let mut tx2 = pool.begin().await.unwrap();
    let is_new2 = dedup.process_if_new_tx(&mut tx2, test_ev).await.unwrap();
    assert!(is_new2);
    tx2.commit().await.unwrap();
    assert!(dedup.is_duplicate(test_ev).await.unwrap());
}

#[tokio::test]
async fn test_per_017_durable_writer_event_recovery_roundtrip() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;
    let writer = DurableWriter::new(pool.clone());

    // 1. Commit MarketPurchase with ItemPurchased and GoldDeducted events
    let cmd_id = Uuid::new_v4();
    let ev1 = Uuid::new_v4();
    let ev2 = Uuid::new_v4();
    let intent = DurableIntent {
        command_id: cmd_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: Some(fixture.actor_id),
        payload_hash: [99u8; 32],
        tick: 50,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 2,
            unit_price: 10,
        }],
        emitted_events: vec![
            DurableEvent {
                event_id: ev1,
                event_type: "ItemPurchased".into(),
                schema_version: 1,
                rules_version: 1,
                tick: 50,
                causation_event_id: None,
                correlation_id: cmd_id,
                payload: serde_json::json!({
                    "buyer_id": fixture.actor_id,
                    "sku": "potion_healing",
                    "quantity": 2,
                    "cost": 20
                }),
                outbox_topic: Some("market.purchased".into()),
            },
            DurableEvent {
                event_id: ev2,
                event_type: "GoldDeducted".into(),
                schema_version: 1,
                rules_version: 1,
                tick: 50,
                causation_event_id: Some(ev1),
                correlation_id: cmd_id,
                payload: serde_json::json!({
                    "wallet_id": fixture.buyer_wallet_id,
                    "amount": 20
                }),
                outbox_topic: None,
            },
        ],
    };
    let ack = writer.commit_intent(intent).await.expect("commit purchase");
    assert_eq!(ack.status, ReceiptStatus::Succeeded);

    // 2. Perform region recovery: should replay financial events cleanly without "Unknown domain event type" error
    let recovered = persistence::RecoveryManager::recover_region(
        &pool,
        fixture.region_id,
        "worker-after-purchase",
        60,
    )
    .await
    .expect("Region recovery must succeed after durable writer transaction");

    assert_eq!(recovered.region_id, fixture.region_id);
    assert_eq!(recovered.tick, 50);
    assert_eq!(recovered.replayed_events_count, 2);
    assert_eq!(recovered.committed_event_seq, 2);
}
