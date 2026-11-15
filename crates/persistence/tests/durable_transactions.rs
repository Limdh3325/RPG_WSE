use persistence::{
    CommandReceiptRecord, DurableEvent, DurableIntent, DurableOperation, DurableTransactionWriter,
    Failpoint, PersistenceError, ReceiptStatus, acquire_or_renew_lease, apply_migration,
    lookup_receipt,
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

const DEFAULT_DEV_DATABASE_URL: &str = "[REDACTED:connection_string]127.0.0.1:55432/wse_dev";

async fn setup_test_pool() -> Option<PgPool> {
    let url = env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DEV_DATABASE_URL.to_string());
    match PgPool::connect(&url).await {
        Ok(pool) => {
            apply_migration(&pool)
                .await
                .expect("Failed to apply initial schema migration");
            Some(pool)
        }
        Err(e) => {
            eprintln!("Skipping PostgreSQL integration tests (cannot connect: {e})");
            None
        }
    }
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
    let settlement_id = region_id; // Settlement shares region id per domain spec

    // 1. World
    sqlx::query("INSERT INTO worlds (id, name) VALUES ($1, $2)")
        .bind(world_id)
        .bind("Test World")
        .execute(pool)
        .await
        .expect("insert world");

    // 2. Region
    sqlx::query("INSERT INTO regions (id, world_id, slug, display_name) VALUES ($1, $2, $3, $4)")
        .bind(region_id)
        .bind(world_id)
        .bind(format!("region-{}", &region_id.to_string()[..8]))
        .bind("Test Region")
        .execute(pool)
        .await
        .expect("insert region");

    // 3. Region Lease (epoch = 1, ttl = 60s)
    let epoch = acquire_or_renew_lease(pool, region_id, "test_worker_1", 60)
        .await
        .expect("acquire lease");

    // 4. Player Entity & Wallet
    sqlx::query("INSERT INTO entities (id, world_id, region_id, kind, name) VALUES ($1, $2, $3, 'PLAYER', 'TestHero')")
        .bind(actor_id)
        .bind(world_id)
        .bind(region_id)
        .execute(pool)
        .await
        .expect("insert entity");

    sqlx::query("INSERT INTO wallets (id, currency_balance, version) VALUES ($1, $2, 1)")
        .bind(actor_id)
        .bind(initial_balance)
        .execute(pool)
        .await
        .expect("insert wallet");

    // 5. Settlement & Market Stock
    sqlx::query("INSERT INTO settlements (id, world_id, slug, name, treasury_balance) VALUES ($1, $2, $3, $4, 0)")
        .bind(settlement_id)
        .bind(world_id)
        .bind(format!("settlement-{}", &settlement_id.to_string()[..8]))
        .bind("Test Town")
        .execute(pool)
        .await
        .expect("insert settlement");

    sqlx::query(
        "INSERT INTO market_stock (settlement_id, sku, quantity, price, target_stock, version) VALUES ($1, $2, $3, 10, 100, 1)"
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

async fn get_wallet_balance(pool: &PgPool, wallet_id: Uuid) -> i64 {
    let (balance,): (i64,) = sqlx::query_as("SELECT currency_balance FROM wallets WHERE id = $1")
        .bind(wallet_id)
        .fetch_one(pool)
        .await
        .expect("fetch wallet");
    balance
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

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
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

    let result =
        DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::BeforeBegin).await;
    assert!(
        result.is_err(),
        "Expected failure due to BeforeBegin failpoint"
    );

    // Assert asset state is untouched
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );

    // Assert no receipt recorded
    let receipt = lookup_receipt(&pool, command_id)
        .await
        .expect("lookup receipt");
    assert!(receipt.is_none());
}

#[tokio::test]
async fn test_per_002_failpoint_after_asset_update_before_commit_rolls_back() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
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

    let result = DurableTransactionWriter::commit_intent(
        &pool,
        &intent,
        Failpoint::AfterAssetUpdateBeforeCommit,
    )
    .await;
    assert!(result.is_err(), "Expected rollback failure");

    // Assert atomic rollback: balance and stock are unchanged!
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );

    // Assert receipt is rolled back as well (not committed)
    let receipt = lookup_receipt(&pool, command_id)
        .await
        .expect("lookup receipt");
    assert!(receipt.is_none(), "Pending receipt should be rolled back");
}

#[tokio::test]
async fn test_per_003_commit_succeeds_ack_lost_lookup_verifies_success() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;

    let command_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
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
            id: event_id,
            stream_type: "TRADE".to_string(),
            aggregate_id: fixture.actor_id,
            event_type: "ItemPurchased".to_string(),
            topic: "domain.events.trade".to_string(),
            payload: serde_json::json!({ "sku": "potion_healing", "quantity": 5 }),
        }],
    };

    // Commit with simulated ACK loss
    let result =
        DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::AfterCommitBeforeAck)
            .await;
    assert!(result.is_err(), "Simulated network drop before ACK return");

    // Verification via lookup_receipt: Transaction actually committed in DB!
    let receipt = lookup_receipt(&pool, command_id)
        .await
        .expect("lookup receipt");
    let record: CommandReceiptRecord = receipt.expect("Receipt must exist after commit");
    assert_eq!(record.status, ReceiptStatus::Succeeded);
    assert_eq!(record.committed_tick, Some(12));

    // Verify assets deducted exactly once
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        950
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        45
    );

    // Verify outbox entry exists
    let (outbox_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM outbox WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .expect("count outbox");
    assert_eq!(outbox_count, 1);
}

#[tokio::test]
async fn test_per_006_lease_fencing_rejects_stale_epoch() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;

    let stale_epoch = fixture.epoch - 1; // Stale epoch
    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: stale_epoch,
        actor_id: fixture.actor_id,
        payload_hash: [6u8; 32],
        tick: 20,
        operations: vec![DurableOperation::WalletTransfer {
            wallet_id: fixture.buyer_wallet_id,
            delta: -100,
        }],
        emitted_events: vec![],
    };

    let result = DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::None).await;
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

    // Assert 0 commits: wallet balance unchanged!
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
}

#[tokio::test]
async fn test_idempotency_100_replays_exactly_one_charge() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 50).await;

    let command_id = Uuid::new_v4();
    let payload_hash = [42u8; 32];
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
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
    let first_ack = DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::None)
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
        let ack = DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::None)
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
async fn test_insufficient_funds_rolls_back_cleanly() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 20, 50).await; // only 20 gold

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
        payload_hash: [7u8; 32],
        tick: 40,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5,
            unit_price: 10, // requires 50 gold
        }],
        emitted_events: vec![],
    };

    let result = DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::None).await;
    match result {
        Err(PersistenceError::InsufficientFunds {
            balance, required, ..
        }) => {
            assert_eq!(balance, 20);
            assert_eq!(required, 50);
        }
        other => panic!("Expected InsufficientFunds, got {other:?}"),
    }

    // Balance remains 20, stock remains 50
    assert_eq!(get_wallet_balance(&pool, fixture.buyer_wallet_id).await, 20);
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        50
    );
}

#[tokio::test]
async fn test_insufficient_stock_rolls_back_cleanly() {
    let Some(pool) = setup_test_pool().await else {
        return;
    };
    let fixture = create_test_fixture(&pool, 1000, 2).await; // only 2 in stock

    let command_id = Uuid::new_v4();
    let intent = DurableIntent {
        command_id,
        world_id: fixture.world_id,
        region_id: fixture.region_id,
        ownership_epoch: fixture.epoch,
        actor_id: fixture.actor_id,
        payload_hash: [8u8; 32],
        tick: 41,
        operations: vec![DurableOperation::MarketPurchase {
            settlement_id: fixture.settlement_id,
            buyer_wallet_id: fixture.buyer_wallet_id,
            seller_wallet_id: None,
            sku: "potion_healing".to_string(),
            quantity: 5, // requests 5
            unit_price: 10,
        }],
        emitted_events: vec![],
    };

    let result = DurableTransactionWriter::commit_intent(&pool, &intent, Failpoint::None).await;
    match result {
        Err(PersistenceError::InsufficientStock {
            available,
            required,
            ..
        }) => {
            assert_eq!(available, 2);
            assert_eq!(required, 5);
        }
        other => panic!("Expected InsufficientStock, got {other:?}"),
    }

    // Balance and stock remain unchanged
    assert_eq!(
        get_wallet_balance(&pool, fixture.buyer_wallet_id).await,
        1000
    );
    assert_eq!(
        get_market_stock(&pool, fixture.settlement_id, "potion_healing").await,
        2
    );
}
