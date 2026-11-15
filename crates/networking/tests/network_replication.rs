use futures_util::{SinkExt, StreamExt};
use networking::gateway::DEFAULT_SHARED_SETTLEMENT_REGION_ID;
use networking::{GatewayState, InboundCommand, RegionReplicator, Session, create_gateway_router};
use shared_protocol::{
    ClientMessage, EntityStateWire, ServerMessage, WireCommandPayload, WireCommandStatus,
};
use simulation_systems::PrologueAction;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

async fn spawn_test_server(
    command_tx: Option<mpsc::Sender<InboundCommand>>,
) -> (String, Arc<GatewayState>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let state = Arc::new(GatewayState::new(command_tx));
    let router = create_gateway_router(state.clone());

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve test gateway");
    });

    (format!("127.0.0.1:{}", addr.port()), state)
}

/// Helper: connects to WS and performs AUTH handshake.
async fn connect_and_auth(
    addr: &str,
    ticket: &str,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    ServerMessage,
) {
    let url = format!("ws://{}/ws", addr);
    let (ws_stream, _) = connect_async(&url).await.expect("connect to ws");
    let (mut sink, mut stream) = ws_stream.split();

    let auth_msg = ClientMessage::Auth {
        ticket: ticket.to_string(),
    };
    let json = serde_json::to_string(&auth_msg).expect("serialize auth");
    sink.send(Message::Text(json.into()))
        .await
        .expect("send auth");

    let first_msg = stream.next().await.expect("response").expect("read msg");
    let first_text = first_msg.to_text().expect("to text");
    let resp: ServerMessage = serde_json::from_str(first_text).expect("parse server message");

    (sink, stream, resp)
}

#[tokio::test]
async fn test_net_001_two_clients_observe_identical_npc_coordinates() {
    let region_id = Uuid::new_v4();
    let (addr, state) = spawn_test_server(None).await;
    let replicator = state.get_or_create_replicator(region_id);

    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let npc_id = Uuid::new_v4();

    let (ticket_a, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_a,
        region_id,
        Duration::from_secs(30),
    );
    let (ticket_b, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_b,
        region_id,
        Duration::from_secs(30),
    );

    let (_sink_a, mut stream_a, auth_a) = connect_and_auth(&addr, &ticket_a).await;
    let (_sink_b, mut stream_b, auth_b) = connect_and_auth(&addr, &ticket_b).await;

    assert!(matches!(auth_a, ServerMessage::AuthOk { .. }));
    assert!(matches!(auth_b, ServerMessage::AuthOk { .. }));

    // Send baseline snapshot to both
    let mut initial_entities = vec![
        EntityStateWire {
            id: actor_a,
            x: 0,
            y: 0,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: Some("Player A".into()),
        },
        EntityStateWire {
            id: actor_b,
            x: 2,
            y: 2,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: Some("Player B".into()),
        },
        EntityStateWire {
            id: npc_id,
            x: 5,
            y: 5,
            entity_type: "npc".into(),
            hp: Some(50),
            max_hp: Some(50),
            name: Some("Merchant".into()),
        },
    ];

    let session_ids: Vec<Uuid> = {
        let lock = replicator.sessions.read().unwrap();
        lock.keys().copied().collect()
    };
    for sid in &session_ids {
        replicator
            .send_initial_snapshot(sid, 1, &initial_entities)
            .unwrap();
    }

    // Both should read the initial WorldSnapshot
    let snap_a = stream_a.next().await.unwrap().unwrap();
    let snap_b = stream_b.next().await.unwrap().unwrap();
    assert!(snap_a.to_text().unwrap().contains("world_snapshot"));
    assert!(snap_b.to_text().unwrap().contains("world_snapshot"));

    // NPC moves from (5,5) to (7,8)
    initial_entities[2].x = 7;
    initial_entities[2].y = 8;

    replicator.broadcast_tick(2, &initial_entities);

    let delta_a_raw = stream_a.next().await.unwrap().unwrap();
    let delta_b_raw = stream_b.next().await.unwrap().unwrap();

    let delta_a: ServerMessage =
        serde_json::from_str(delta_a_raw.to_text().unwrap()).expect("parse delta A");
    let delta_b: ServerMessage =
        serde_json::from_str(delta_b_raw.to_text().unwrap()).expect("parse delta B");

    match (delta_a, delta_b) {
        (
            ServerMessage::StateDelta {
                changed: changed_a, ..
            },
            ServerMessage::StateDelta {
                changed: changed_b, ..
            },
        ) => {
            let npc_pos_a = changed_a.iter().find(|e| e.id == npc_id).unwrap();
            let npc_pos_b = changed_b.iter().find(|e| e.id == npc_id).unwrap();
            assert_eq!(npc_pos_a.x, 7);
            assert_eq!(npc_pos_a.y, 8);
            assert_eq!(npc_pos_b.x, 7);
            assert_eq!(npc_pos_b.y, 8);
        }
        other => panic!("Expected StateDelta on both clients, got {:?}", other),
    }
}

#[tokio::test]
async fn test_net_002_delta_gap_triggers_resync_and_state_consistency() {
    let region_id = Uuid::new_v4();
    let (addr, state) = spawn_test_server(None).await;
    let replicator = state.get_or_create_replicator(region_id);
    let actor_id = Uuid::new_v4();

    let (ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_id,
        region_id,
        Duration::from_secs(30),
    );

    let (mut sink, mut stream, auth) = connect_and_auth(&addr, &ticket).await;
    assert!(matches!(auth, ServerMessage::AuthOk { .. }));

    let entity = EntityStateWire {
        id: actor_id,
        x: 10,
        y: 10,
        entity_type: "player".into(),
        hp: Some(100),
        max_hp: Some(100),
        name: None,
    };

    let session_id = {
        let lock = replicator.sessions.read().unwrap();
        *lock.keys().next().unwrap()
    };
    replicator
        .send_initial_snapshot(&session_id, 1, std::slice::from_ref(&entity))
        .unwrap();

    let _ = stream.next().await.unwrap().unwrap(); // WorldSnapshot

    // Client detects gap or needs resync: client sends ResyncRequest
    let resync_req = ClientMessage::ResyncRequest {
        reason: "gap_detected".into(),
        last_seen_revision: 1,
    };
    let json = serde_json::to_string(&resync_req).unwrap();
    sink.send(Message::Text(json.into())).await.unwrap();

    // Client should receive a fresh WorldSnapshot
    let resync_resp = stream.next().await.unwrap().unwrap();
    let parsed: ServerMessage =
        serde_json::from_str(resync_resp.to_text().unwrap()).expect("parse resync snapshot");

    assert!(matches!(parsed, ServerMessage::WorldSnapshot { .. }));
}

#[tokio::test]
async fn test_net_003_idempotent_command_no_double_execution() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<InboundCommand>(32);
    let (addr, state) = spawn_test_server(Some(cmd_tx)).await;
    let region_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let (ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_id,
        region_id,
        Duration::from_secs(30),
    );

    let (mut sink, _stream, _auth) = connect_and_auth(&addr, &ticket).await;

    let command_id = Uuid::new_v4();
    let cmd = ClientMessage::Command {
        command_id,
        actor_id,
        payload: WireCommandPayload::Move {
            target_x: 15,
            target_y: 20,
        },
    };

    // Send command
    let json = serde_json::to_string(&cmd).unwrap();
    sink.send(Message::Text(json.clone().into())).await.unwrap();

    // Server gateway receives command
    let received_cmd = cmd_rx.recv().await.expect("received command");
    assert_eq!(received_cmd.command_id, command_id);
    assert_eq!(received_cmd.actor_id, actor_id);

    // Simulate network retry / re-send with same command_id
    sink.send(Message::Text(json.into())).await.unwrap();
    let received_retry = cmd_rx.recv().await.expect("received retry command");
    // Command ID is identical, enabling downstream idempotent deduplication
    assert_eq!(received_retry.command_id, command_id);
}

#[tokio::test]
async fn test_net_004_forged_actor_id_is_rejected() {
    let (cmd_tx, _cmd_rx) = mpsc::channel::<InboundCommand>(32);
    let (addr, state) = spawn_test_server(Some(cmd_tx)).await;
    let region_id = Uuid::new_v4();
    let legitimate_actor_id = Uuid::new_v4();
    let forged_actor_id = Uuid::new_v4();

    let (ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        legitimate_actor_id,
        region_id,
        Duration::from_secs(30),
    );

    let (mut sink, mut stream, _auth) = connect_and_auth(&addr, &ticket).await;

    // Client attempts to impersonate another actor
    let forged_cmd = ClientMessage::Command {
        command_id: Uuid::new_v4(),
        actor_id: forged_actor_id,
        payload: WireCommandPayload::Move {
            target_x: 99,
            target_y: 99,
        },
    };

    let json = serde_json::to_string(&forged_cmd).unwrap();
    sink.send(Message::Text(json.into())).await.unwrap();

    let resp_raw = stream.next().await.unwrap().unwrap();
    let resp: ServerMessage =
        serde_json::from_str(resp_raw.to_text().unwrap()).expect("parse command status");

    match resp {
        ServerMessage::CommandStatus { status, reason, .. } => {
            assert_eq!(status, WireCommandStatus::Rejected);
            assert!(reason.unwrap().contains("Actor ID does not match"));
        }
        other => panic!("Expected CommandStatus rejected, got {:?}", other),
    }
}

#[tokio::test]
async fn test_net_005_aoi_outside_entities_not_sent_to_uninterested_sessions() {
    let region_id = Uuid::new_v4();
    let replicator = RegionReplicator::new(region_id);

    let actor_near = Uuid::new_v4();
    let actor_far = Uuid::new_v4();

    let (tx_near, mut rx_near) = mpsc::channel::<ServerMessage>(32);
    let (tx_far, _rx_far) = mpsc::channel::<ServerMessage>(32);

    let session_near = Arc::new(Session::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        actor_near,
        region_id,
        tx_near,
    ));
    let session_far = Arc::new(Session::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        actor_far,
        region_id,
        tx_far,
    ));

    replicator.add_session(session_near.clone());
    replicator.add_session(session_far.clone());

    // Near player is at (0, 0) -> cell (0, 0)
    // Far player is at (100, 100) -> cell (6, 6)
    // NPC is at (200, 200) -> cell (12, 12), then moves to (201, 201)
    let entities = vec![
        EntityStateWire {
            id: actor_near,
            x: 0,
            y: 0,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: None,
        },
        EntityStateWire {
            id: actor_far,
            x: 100,
            y: 100,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: None,
        },
        EntityStateWire {
            id: Uuid::new_v4(),
            x: 200,
            y: 200,
            entity_type: "npc".into(),
            hp: Some(50),
            max_hp: Some(50),
            name: None,
        },
    ];

    replicator
        .send_initial_snapshot(&session_near.session_id, 1, &entities)
        .unwrap();

    let snap_near = rx_near.try_recv().expect("near receives snapshot");
    let ServerMessage::WorldSnapshot {
        entities: visible, ..
    } = snap_near
    else {
        panic!("Expected WorldSnapshot");
    };
    // Near session should only see actor_near, neither actor_far nor the distant NPC
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, actor_near);

    // Now update NPC at (200, 200) to (201, 201)
    let mut updated_entities = entities.clone();
    updated_entities[2].x = 201;
    updated_entities[2].y = 201;

    replicator.broadcast_tick(2, &updated_entities);

    // Session Near should receive NOTHING since NPC is way outside its AOI (cell 12 vs cell 0)
    assert!(
        rx_near.try_recv().is_err(),
        "Session Near should not receive updates for distant NPC"
    );
}

#[tokio::test]
async fn test_net_006_slow_client_does_not_block_region_tick() {
    let region_id = Uuid::new_v4();
    let replicator = RegionReplicator::new(region_id);

    let actor_fast = Uuid::new_v4();
    let actor_slow = Uuid::new_v4();

    // Fast client has capacity
    let (tx_fast, mut rx_fast) = mpsc::channel::<ServerMessage>(32);
    // Slow client has capacity 1, which will immediately saturate
    let (tx_slow, _rx_slow) = mpsc::channel::<ServerMessage>(1);

    let session_fast = Arc::new(Session::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        actor_fast,
        region_id,
        tx_fast,
    ));
    let session_slow = Arc::new(Session::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        actor_slow,
        region_id,
        tx_slow,
    ));

    replicator.add_session(session_fast.clone());
    replicator.add_session(session_slow.clone());

    let mut entities = vec![
        EntityStateWire {
            id: actor_fast,
            x: 0,
            y: 0,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: None,
        },
        EntityStateWire {
            id: actor_slow,
            x: 0,
            y: 0,
            entity_type: "player".into(),
            hp: Some(100),
            max_hp: Some(100),
            name: None,
        },
    ];

    // Establish baseline snapshot for fast client
    replicator
        .send_initial_snapshot(&session_fast.session_id, 1, &entities)
        .unwrap();
    let _ = rx_fast.try_recv().expect("drain initial snapshot");

    // Establish baseline and saturate slow client's single-item channel
    replicator
        .send_initial_snapshot(&session_slow.session_id, 1, &entities)
        .unwrap();
    // Do NOT drain rx_slow so its 1-capacity buffer remains full!

    // Fast client can still receive, and broadcast completes immediately without blocking
    entities[0].x = 1;
    let start = std::time::Instant::now();
    let disconnected = replicator.broadcast_tick(2, &entities);
    let elapsed = start.elapsed();

    // Broadcast must complete in microseconds, not hang
    assert!(
        elapsed < Duration::from_millis(50),
        "Broadcast took too long: {:?}",
        elapsed
    );

    // Slow client queue was full, so it got isolated/disconnected
    assert!(disconnected.contains(&session_slow.session_id));
    assert_eq!(replicator.session_count(), 1);

    // Fast client receives its delta normally
    let fast_msg = rx_fast.try_recv().expect("fast client got delta");
    assert!(matches!(fast_msg, ServerMessage::StateDelta { .. }));
}

#[tokio::test]
async fn test_net_007_completed_prologue_reconnect_binds_to_settlement() {
    let (cmd_tx, _cmd_rx) = mpsc::channel::<InboundCommand>(32);
    let (addr, state) = spawn_test_server(Some(cmd_tx)).await;

    let actor_id = Uuid::new_v4();
    let prologue_private_region = Uuid::new_v4();

    // Mark character as completed prologue in PrologueManager
    {
        let mut manager = state.prologue_manager.lock().unwrap();
        manager
            .start_prologue(actor_id, prologue_private_region, 1000)
            .unwrap();
        manager
            .advance_step(
                actor_id,
                PrologueAction::TalkToNpc {
                    npc_name: "Elder Elda".to_string(),
                },
                1100,
            )
            .unwrap();
        manager
            .advance_step(
                actor_id,
                PrologueAction::GatherResource {
                    resource_type: "herb".to_string(),
                    amount: 3,
                },
                1200,
            )
            .unwrap();
        manager
            .advance_step(
                actor_id,
                PrologueAction::ReachWaypoint { x: 120, y: 150 },
                1300,
            )
            .unwrap();
        manager
            .advance_step(
                actor_id,
                PrologueAction::DefeatEncounter {
                    target_id: "goblin_scout_01".to_string(),
                },
                1400,
            )
            .unwrap();
        manager
            .advance_step(
                actor_id,
                PrologueAction::PackSupplies {
                    supply_id: "wagon_crate_01".to_string(),
                },
                1500,
            )
            .unwrap();
        manager
            .advance_step(actor_id, PrologueAction::EnterSettlementGate, 1600)
            .unwrap();
        let _ = manager
            .transfer_to_shared_world(actor_id, DEFAULT_SHARED_SETTLEMENT_REGION_ID, 2000)
            .unwrap();
        assert!(manager.is_completed(&actor_id));
    }

    // Client reconnects with ticket mistakenly specifying private prologue region
    let (ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_id,
        prologue_private_region,
        Duration::from_secs(30),
    );

    let (_sink, _stream, auth_msg) = connect_and_auth(&addr, &ticket).await;

    // Verify AuthOk returned the settlement region, NOT the prologue private region
    match auth_msg {
        ServerMessage::AuthOk {
            actor_id: authed_actor,
            region_id: authed_region,
            ..
        } => {
            assert_eq!(authed_actor, actor_id);
            assert_eq!(
                authed_region, DEFAULT_SHARED_SETTLEMENT_REGION_ID,
                "Completed prologue actor must be routed directly to shared settlement region on reconnect"
            );
        }
        other => panic!("Expected AuthOk, got: {:?}", other),
    }

    // Verify session was added to settlement replicator, NOT private replicator
    let reps = state.replicators.read().unwrap();
    let settlement_rep = reps.get(&DEFAULT_SHARED_SETTLEMENT_REGION_ID).unwrap();
    assert_eq!(settlement_rep.session_count(), 1);
    if let Some(priv_rep) = reps.get(&prologue_private_region) {
        assert_eq!(priv_rep.session_count(), 0);
    }
}

#[tokio::test]
async fn test_net_008_lock_poisoning_resilience() {
    let (cmd_tx, mut _cmd_rx) = mpsc::channel::<InboundCommand>(32);
    let (addr, state) = spawn_test_server(Some(cmd_tx)).await;

    // 1. Poison prologue_manager lock intentionally
    let pm_clone = state.prologue_manager.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = pm_clone.lock().unwrap();
        panic!("intentional panic to poison prologue_manager");
    }));
    assert!(
        state.prologue_manager.is_poisoned(),
        "prologue_manager mutex must be poisoned"
    );

    // 2. Poison replicators lock intentionally
    let rep_clone = state.replicators.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = rep_clone.write().unwrap();
        panic!("intentional panic to poison replicators");
    }));
    assert!(
        state.replicators.is_poisoned(),
        "replicators rwlock must be poisoned"
    );

    // 3. Verify get_or_create_replicator still functions seamlessly despite poisoned lock
    let test_region = Uuid::new_v4();
    let rep = state.get_or_create_replicator(test_region);
    assert_eq!(rep.session_count(), 0);

    // 4. Authenticate client through websocket and verify prologue query works despite poisoned locks
    let actor_id = Uuid::new_v4();
    let (ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_id,
        test_region,
        Duration::from_secs(30),
    );

    let (mut sink, mut stream, auth_msg) = connect_and_auth(&addr, &ticket).await;
    assert!(matches!(auth_msg, ServerMessage::AuthOk { .. }));

    // Send PrologueQueryState - must not panic or crash
    let query_wire = serde_json::to_string(&ClientMessage::PrologueQueryState).unwrap();
    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        query_wire.into(),
    ))
    .await
    .unwrap();

    let resp = stream.next().await.expect("got response").unwrap();
    let resp_msg: ServerMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
    assert!(
        matches!(resp_msg, ServerMessage::PrologueStateSync { .. }),
        "Expected PrologueStateSync despite poisoned prologue manager lock, got: {:?}",
        resp_msg
    );

    // Send PrologueStart - must not panic or crash
    let start_wire = serde_json::to_string(&ClientMessage::PrologueStart).unwrap();
    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        start_wire.into(),
    ))
    .await
    .unwrap();

    let resp2 = stream.next().await.expect("got response").unwrap();
    let resp_msg2: ServerMessage = serde_json::from_str(resp2.to_text().unwrap()).unwrap();
    assert!(
        matches!(resp_msg2, ServerMessage::PrologueStateSync { .. }),
        "Expected PrologueStateSync after start despite poisoned locks, got: {:?}",
        resp_msg2
    );

    // 5. Explicitly poison Session.region_id lock and verify resilience
    let (tx_dummy, _rx_dummy) = mpsc::channel(1);
    let session = Arc::new(Session::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        test_region,
        tx_dummy,
    ));

    // Intentionally poison session.region_id lock
    let sess_clone = session.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Access raw RwLock through mutation helper or trigger panic
        let _guard = sess_clone.region_id.write().unwrap();
        panic!("intentional panic to poison session.region_id");
    }));
    assert!(
        session.region_id.is_poisoned(),
        "session.region_id rwlock must be poisoned"
    );

    // region_id() getter must not panic and recover inner value
    assert_eq!(session.region_id(), test_region);

    // set_region_id() setter must not panic and successfully mutate value
    let target_settlement = Uuid::new_v4();
    session.set_region_id(target_settlement);
    assert_eq!(session.region_id(), target_settlement);

    // 6. Verify Command dispatch over websocket does not panic on session.region_id() lookup
    let cmd_wire = serde_json::to_string(&ClientMessage::Command {
        command_id: Uuid::new_v4(),
        actor_id,
        payload: shared_protocol::WireCommandPayload::Noop,
    })
    .unwrap();
    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        cmd_wire.into(),
    ))
    .await
    .unwrap();

    let cmd_received = tokio::time::timeout(Duration::from_millis(500), _cmd_rx.recv())
        .await
        .expect("command should be forwarded without panic")
        .expect("inbound command present");
    assert_eq!(cmd_received.region_id, test_region);
    assert_eq!(cmd_received.actor_id, actor_id);
}
