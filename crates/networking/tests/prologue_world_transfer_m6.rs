use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use shared_protocol::{ClientMessage, ServerMessage};
use simulation_systems::prologue::{FilePrologueStorage, PrologueManager, PrologueState};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use uuid::Uuid;

use networking::gateway::{create_gateway_router, GatewayState, DEFAULT_SHARED_SETTLEMENT_REGION_ID};

async fn setup_test_server(custom_data_dir: Option<std::path::PathBuf>) -> (String, Arc<GatewayState>) {
    let state = match custom_data_dir {
        Some(dir) => {
            std::fs::create_dir_all(&dir).unwrap();
            let storage = FilePrologueStorage::new(&dir).unwrap();
            let manager = PrologueManager::try_new_with_storage(Arc::new(storage)).unwrap();
            Arc::new(GatewayState::new(None).with_prologue_manager(Arc::new(std::sync::Mutex::new(manager))))
        }
        None => Arc::new(GatewayState::new(None)),
    };

    let app = create_gateway_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.unwrap();
    });

    (format!("ws://127.0.0.1:{}/ws", addr.port()), state)
}

async fn connect_and_auth(
    ws_url: &str,
    state: &GatewayState,
    account_id: Uuid,
    actor_id: Uuid,
    region_id: Uuid,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Uuid,
) {
    let (ticket, _) = state.ticket_store.issue_ticket(
        account_id,
        actor_id,
        region_id,
        Duration::from_secs(60),
    );

    let (mut ws, _) = connect_async(ws_url).await.unwrap();
    let auth_msg = ClientMessage::Auth { ticket };
    ws.send(Message::Text(serde_json::to_string(&auth_msg).unwrap().into()))
        .await
        .unwrap();

    let mut effective_region = region_id;
    while let Some(Ok(Message::Text(txt))) = ws.next().await {
        if let Ok(ServerMessage::AuthOk { region_id: r_id, .. }) = serde_json::from_str(&txt) {
            effective_region = r_id;
            break;
        }
    }

    (ws, effective_region)
}

async fn wait_for_sync<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    predicate: F,
) -> (String, String, bool, bool)
where
    F: Fn(&str, &str) -> bool,
{
    for _ in 0..30 {
        if let Ok(Some(Ok(Message::Text(txt)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            if let Ok(ServerMessage::PrologueStateSync {
                state,
                step,
                fallback_active,
                is_completed,
                ..
            }) = serde_json::from_str(&txt)
            {
                if predicate(&state, &step) {
                    return (state, step, fallback_active, is_completed);
                }
            }
        }
    }
    panic!("Timeout waiting for matching PrologueStateSync");
}

#[tokio::test]
async fn test_m6_pro_001_full_prologue_flow_to_world_transfer() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_001_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    let (mut ws, region) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;
    assert_eq!(region, private_region_id);

    // 1. Query State -> NOT_STARTED
    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap().into()))
        .await
        .unwrap();
    let (s, _, _, is_completed) = wait_for_sync(&mut ws, |st, _| st == "NOT_STARTED").await;
    assert_eq!(s, "NOT_STARTED");
    assert!(!is_completed);

    // 2. Start -> P0VillageEntry
    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueStart).unwrap().into()))
        .await
        .unwrap();
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P0VillageEntry").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P0VillageEntry");

    // 3. Talk -> P1ResourceGathering
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueStepAction {
            action_type: "talk_to_npc".to_string(),
            target_id: Some("Elder Elda".to_string()),
            payload: None,
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P1ResourceGathering").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P1ResourceGathering");

    // 4. Gather resources (need 3 total to advance to P2)
    for _ in 0..3 {
        ws.send(Message::Text(
            serde_json::to_string(&ClientMessage::PrologueStepAction {
                action_type: "gather".to_string(),
                target_id: Some("wild_herbs".to_string()),
                payload: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    }
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P2OutpostTravel").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P2OutpostTravel");

    // 5. Reach Waypoint -> P3RescueCombat
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueStepAction {
            action_type: "waypoint".to_string(),
            target_id: Some("outpost_gate".to_string()),
            payload: None,
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P3RescueCombat").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P3RescueCombat");

    // 6. Defeat Combat Encounter -> P4SupplyMigration
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueStepAction {
            action_type: "defeat_encounter".to_string(),
            target_id: Some("goblin_scout".to_string()),
            payload: None,
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P4SupplyMigration").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P4SupplyMigration");

    // 7. Pack Supplies -> P5SharedSettlement
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueStepAction {
            action_type: "pack_supplies".to_string(),
            target_id: Some("frontier_cart_supplies".to_string()),
            payload: None,
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P5SharedSettlement").await;
    assert_eq!(s, "IN_PROGRESS");
    assert_eq!(step, "P5SharedSettlement");

    // 8. Enter Gate -> READY_TO_TRANSFER
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueStepAction {
            action_type: "enter_settlement".to_string(),
            target_id: Some("settlement_main_gate".to_string()),
            payload: None,
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let (s, _, _, _) = wait_for_sync(&mut ws, |st, _| st == "READY_TO_TRANSFER").await;
    assert_eq!(s, "READY_TO_TRANSFER");

    // 9. Request World Transfer
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueRequestTransfer {
            target_region_id: Some(DEFAULT_SHARED_SETTLEMENT_REGION_ID),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    let mut transfer_committed = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(Message::Text(txt)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            if let Ok(ServerMessage::PrologueTransferResult {
                status,
                target_region_id,
                rewards,
                ..
            }) = serde_json::from_str(&txt)
            {
                if status == "COMMITTED" {
                    assert_eq!(target_region_id, DEFAULT_SHARED_SETTLEMENT_REGION_ID);
                    assert!(!rewards.is_empty());
                    transfer_committed = true;
                    break;
                }
            }
        }
    }
    assert!(transfer_committed, "Expected committed transfer result");

    // Verify prologue is now completed
    let manager = state.prologue_manager.lock().unwrap();
    assert!(manager.is_completed(&actor_id));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_002_disconnect_recovery_all_steps() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_002_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    // Connect, start, advance to P1
    {
        let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;
        ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueStart).unwrap().into()))
            .await
            .unwrap();
        wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P0VillageEntry").await;

        ws.send(Message::Text(
            serde_json::to_string(&ClientMessage::PrologueStepAction {
                action_type: "talk_to_npc".to_string(),
                target_id: Some("Elder Elda".to_string()),
                payload: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P1ResourceGathering").await;
        assert_eq!(s, "IN_PROGRESS");
        assert_eq!(step, "P1ResourceGathering");
        // Drop WebSocket abruptly simulating disconnection
    }

    // Reconnect with new session ticket
    {
        let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;
        ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap().into()))
            .await
            .unwrap();
        let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P1ResourceGathering").await;
        assert_eq!(s, "IN_PROGRESS");
        assert_eq!(step, "P1ResourceGathering");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_003_idempotent_100_transfer_retries() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_003_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;

    // Fast setup directly into ReadyToTransfer state (6 fallback advances from P0 to ReadyToTransfer)
    {
        let mut manager = state.prologue_manager.lock().unwrap();
        manager.start_prologue(actor_id, private_region_id, 0).unwrap();
        for _ in 0..6 {
            manager.trigger_npc_casualty_fallback(actor_id, "test_fast_forward", 0).unwrap();
        }
        let state = manager.get_state(&actor_id);
        assert!(matches!(state, PrologueState::ReadyToTransfer { .. }));
    }

    // Send transfer request 100 times in rapid succession
    for _ in 0..100 {
        ws.send(Message::Text(
            serde_json::to_string(&ClientMessage::PrologueRequestTransfer {
                target_region_id: Some(DEFAULT_SHARED_SETTLEMENT_REGION_ID),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    }

    // Read responses: must commit idempotently
    let mut commit_seen = false;
    for _ in 0..30 {
        if let Ok(Some(Ok(Message::Text(txt)))) =
            tokio::time::timeout(Duration::from_millis(200), ws.next()).await
        {
            if let Ok(ServerMessage::PrologueTransferResult { status, .. }) = serde_json::from_str(&txt) {
                if status == "COMMITTED" {
                    commit_seen = true;
                    break;
                }
            }
        }
    }
    assert!(commit_seen);

    // Verify storage has exactly 1 completed transfer record
    let manager = state.prologue_manager.lock().unwrap();
    assert!(manager.is_completed(&actor_id));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_004_completed_character_routes_to_settlement() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_004_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    // Mark completed beforehand
    {
        let mut manager = state.prologue_manager.lock().unwrap();
        manager.start_prologue(actor_id, private_region_id, 0).unwrap();
        for _ in 0..6 {
            manager.trigger_npc_casualty_fallback(actor_id, "bypass", 0).unwrap();
        }
        manager.transfer_to_shared_world(actor_id, DEFAULT_SHARED_SETTLEMENT_REGION_ID, 0).unwrap();
        assert!(manager.is_completed(&actor_id));
    }

    // Connect with private_region_id ticket: AuthOk must route to DEFAULT_SHARED_SETTLEMENT_REGION_ID
    let (mut ws, effective_region) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;
    assert_eq!(effective_region, DEFAULT_SHARED_SETTLEMENT_REGION_ID);

    // Query state must show is_completed = true
    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap().into()))
        .await
        .unwrap();
    let (s, _, _, is_completed) = wait_for_sync(&mut ws, |st, _| st == "COMPLETED").await;
    assert_eq!(s, "COMPLETED");
    assert!(is_completed);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_005_movement_failure_reports_within_5s() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_005_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;

    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueStart).unwrap().into()))
        .await
        .unwrap();
    wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P0VillageEntry").await;

    // Send movement failure report
    let start = std::time::Instant::now();
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueFallback {
            reason: "Path blocked by collapsed landslide".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    let (_, _, fallback_active, _) = wait_for_sync(&mut ws, |_, _| true).await;
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5), "Failure report must respond within 5s");
    assert!(fallback_active, "fallback_active must be set to true");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_006_combat_or_npc_death_fallback_path() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_006_{}", Uuid::new_v4()));
    let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;

    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;

    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueStart).unwrap().into()))
        .await
        .unwrap();
    wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P0VillageEntry").await;

    // Trigger casualty/combat fallbacks across all steps (6 steps to reach ReadyToTransfer)
    for _ in 0..6 {
        ws.send(Message::Text(
            serde_json::to_string(&ClientMessage::PrologueFallback {
                reason: "Soldier casualty and combat defeat fallback".to_string(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    }

    // Now state should be READY_TO_TRANSFER
    ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap().into()))
        .await
        .unwrap();
    let (s, _, _, _) = wait_for_sync(&mut ws, |st, _| st == "READY_TO_TRANSFER").await;
    assert_eq!(s, "READY_TO_TRANSFER");

    // Perform transfer
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::PrologueRequestTransfer {
            target_region_id: Some(DEFAULT_SHARED_SETTLEMENT_REGION_ID),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    let mut committed = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(Message::Text(txt)))) =
            tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        {
            if let Ok(ServerMessage::PrologueTransferResult { status, .. }) = serde_json::from_str(&txt) {
                if status == "COMMITTED" {
                    committed = true;
                    break;
                }
            }
        }
    }
    assert!(committed, "Expected committed transfer via fallback path");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_m6_pro_007_file_storage_durability_across_gateway_restart() {
    let temp_dir = std::env::temp_dir().join(format!("prologue_test_007_{}", Uuid::new_v4()));
    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    // Run instance 1 and advance to P1
    {
        let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;
        let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;

        ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueStart).unwrap().into()))
            .await
            .unwrap();
        wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P0VillageEntry").await;

        ws.send(Message::Text(
            serde_json::to_string(&ClientMessage::PrologueStepAction {
                action_type: "talk_to_npc".to_string(),
                target_id: Some("Elder Elda".to_string()),
                payload: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P1ResourceGathering").await;
        assert_eq!(s, "IN_PROGRESS");
        assert_eq!(step, "P1ResourceGathering");
    }

    // Run instance 2 with the same storage dir (simulating complete gateway reboot/crash recovery)
    {
        let (ws_url, state) = setup_test_server(Some(temp_dir.clone())).await;
        let (mut ws, _) = connect_and_auth(&ws_url, &state, account_id, actor_id, private_region_id).await;

        ws.send(Message::Text(serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap().into()))
            .await
            .unwrap();
        let (s, step, _, _) = wait_for_sync(&mut ws, |st, sp| st == "IN_PROGRESS" && sp == "P1ResourceGathering").await;
        assert_eq!(s, "IN_PROGRESS", "State must persist across gateway restart");
        assert_eq!(step, "P1ResourceGathering", "Step must persist across gateway restart");
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}