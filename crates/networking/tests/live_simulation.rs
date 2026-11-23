use futures_util::{SinkExt, StreamExt};
use networking::{
    DEFAULT_SETTLEMENT_REGION_ID, GatewayState, create_gateway_router,
    simulation::TRAINING_DUMMY_ID,
};
use shared_protocol::{
    ClientMessage, PROTOCOL_VERSION, ServerMessage, WireCommandPayload, WireCommandStatus,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use uuid::Uuid;

async fn spawn_test_server() -> (SocketAddr, Arc<GatewayState>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state =
        Arc::new(GatewayState::default().with_settlement_simulation(DEFAULT_SETTLEMENT_REGION_ID));
    let app = create_gateway_router(state.clone());

    let state_for_tick = state.clone();
    let tick_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        loop {
            interval.tick().await;
            if let Some(ref sim) = state_for_tick.simulation {
                sim.step_tick();
            }
        }
    });

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, state, tick_handle)
}

async fn connect_ws(
    addr: &SocketAddr,
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
    Uuid, // session_id
) {
    let ws_url = format!("ws://{}/ws", addr);
    let (ws_stream, _) = connect_async(&ws_url).await.unwrap();
    let (mut sink, mut stream) = ws_stream.split();

    // 1. Send auth message
    let auth_msg = ClientMessage::Auth {
        ticket: ticket.to_string(),
    };
    sink.send(Message::Text(
        serde_json::to_string(&auth_msg).unwrap().into(),
    ))
    .await
    .unwrap();

    // 2. Expect AuthOk
    let auth_reply = stream.next().await.unwrap().unwrap();
    let session_id =
        match serde_json::from_str::<ServerMessage>(&auth_reply.to_text().unwrap()).unwrap() {
            ServerMessage::AuthOk {
                session_id,
                protocol_version,
                ..
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                session_id
            }
            other => panic!("Expected AuthOk, got {:?}", other),
        };

    (sink, stream, session_id)
}

#[tokio::test]
async fn test_m4_live_two_clients_world_observation_and_combat() {
    let (addr, state, tick_handle) = spawn_test_server().await;

    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let region_id = DEFAULT_SETTLEMENT_REGION_ID;

    // 1. Client A Connects
    let (ticket_a, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_a,
        region_id,
        Duration::from_secs(30),
    );
    let (mut sink_a, mut stream_a, _session_a) = connect_ws(&addr, &ticket_a).await;

    // Client A receives initial WorldSnapshot immediately after auth
    let snap_msg_a = stream_a.next().await.unwrap().unwrap();
    let snap_a =
        match serde_json::from_str::<ServerMessage>(&snap_msg_a.to_text().unwrap()).unwrap() {
            ServerMessage::WorldSnapshot { entities, .. } => entities,
            other => panic!("Expected WorldSnapshot for Client A, got {:?}", other),
        };

    // Verify Training Dummy and Client A are present in world snapshot
    assert!(snap_a.iter().any(|e| e.id == actor_a));
    assert!(snap_a.iter().any(|e| e.id == TRAINING_DUMMY_ID));

    // 2. Client B Connects
    let (ticket_b, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_b,
        region_id,
        Duration::from_secs(30),
    );
    let (mut _sink_b, mut stream_b, _session_b) = connect_ws(&addr, &ticket_b).await;

    // Client B receives initial WorldSnapshot
    let snap_msg_b = stream_b.next().await.unwrap().unwrap();
    let snap_b =
        match serde_json::from_str::<ServerMessage>(&snap_msg_b.to_text().unwrap()).unwrap() {
            ServerMessage::WorldSnapshot { entities, .. } => entities,
            other => panic!("Expected WorldSnapshot for Client B, got {:?}", other),
        };

    // Client B sees both Client A and Client B
    assert!(snap_b.iter().any(|e| e.id == actor_a));
    assert!(snap_b.iter().any(|e| e.id == actor_b));

    // 3. Client A moves to (10, 10)
    let move_cmd_id = Uuid::new_v4();
    let move_cmd = ClientMessage::Command {
        command_id: move_cmd_id,
        actor_id: actor_a,
        payload: WireCommandPayload::Move {
            target_x: 10,
            target_y: 10,
        },
    };
    sink_a
        .send(Message::Text(
            serde_json::to_string(&move_cmd).unwrap().into(),
        ))
        .await
        .unwrap();

    // Client A receives CommandStatus::Committed
    let mut found_committed = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(100), stream_a.next()).await
        {
            let text = msg.to_text().unwrap();
            if let Ok(ServerMessage::CommandStatus {
                command_id, status, ..
            }) = serde_json::from_str::<ServerMessage>(text)
            {
                if command_id == move_cmd_id && status == WireCommandStatus::Committed {
                    found_committed = true;
                    break;
                }
            }
        }
    }
    assert!(found_committed, "Client A move command was not committed");

    // 4. Client B receives StateDelta showing Client A at (10, 10)
    let mut client_b_observed_move = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(100), stream_b.next()).await
        {
            let text = msg.to_text().unwrap();
            if let Ok(ServerMessage::StateDelta { changed, .. }) =
                serde_json::from_str::<ServerMessage>(text)
            {
                for entity in changed {
                    if entity.id == actor_a && entity.x == 10 && entity.y == 10 {
                        client_b_observed_move = true;
                        break;
                    }
                }
                if client_b_observed_move {
                    break;
                }
            }
        }
    }
    assert!(
        client_b_observed_move,
        "Client B did not observe Client A's move delta"
    );

    // 5. Client A attacks Training Dummy
    let attack_cmd_id = Uuid::new_v4();
    let attack_cmd = ClientMessage::Command {
        command_id: attack_cmd_id,
        actor_id: actor_a,
        payload: WireCommandPayload::Attack {
            target_id: TRAINING_DUMMY_ID,
        },
    };
    sink_a
        .send(Message::Text(
            serde_json::to_string(&attack_cmd).unwrap().into(),
        ))
        .await
        .unwrap();

    let mut attack_committed = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(100), stream_a.next()).await
        {
            let text = msg.to_text().unwrap();
            if let Ok(ServerMessage::CommandStatus {
                command_id, status, ..
            }) = serde_json::from_str::<ServerMessage>(text)
            {
                if command_id == attack_cmd_id && status == WireCommandStatus::Committed {
                    attack_committed = true;
                    break;
                }
            }
        }
    }
    assert!(
        attack_committed,
        "Client A attack command was not committed"
    );

    // Client B observes Training Dummy HP reduced to 75
    let mut client_b_observed_damage = false;
    for _ in 0..20 {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(100), stream_b.next()).await
        {
            let text = msg.to_text().unwrap();
            if let Ok(ServerMessage::StateDelta { changed, .. }) =
                serde_json::from_str::<ServerMessage>(text)
            {
                for entity in changed {
                    if entity.id == TRAINING_DUMMY_ID && entity.hp == Some(75) {
                        client_b_observed_damage = true;
                        break;
                    }
                }
                if client_b_observed_damage {
                    break;
                }
            }
        }
    }
    assert!(
        client_b_observed_damage,
        "Client B did not observe damage dealt to Training Dummy"
    );

    // 6. Client A Disconnects & Reconnects
    drop(sink_a);
    drop(stream_a);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Issue new ticket for Actor A and reconnect
    let (reconnect_ticket, _) = state.ticket_store.issue_ticket(
        Uuid::new_v4(),
        actor_a,
        region_id,
        Duration::from_secs(30),
    );
    let (_sink_reconnect, mut stream_reconnect, _) = connect_ws(&addr, &reconnect_ticket).await;

    let snap_reconnect_msg = stream_reconnect.next().await.unwrap().unwrap();
    let snap_reconnect = match serde_json::from_str::<ServerMessage>(
        &snap_reconnect_msg.to_text().unwrap(),
    )
    .unwrap()
    {
        ServerMessage::WorldSnapshot { entities, .. } => entities,
        other => panic!("Expected WorldSnapshot on reconnect, got {:?}", other),
    };

    // Verify actor A's position is still at (10, 10) and dummy HP is 75
    let restored_a = snap_reconnect.iter().find(|e| e.id == actor_a).unwrap();
    assert_eq!(restored_a.x, 10);
    assert_eq!(restored_a.y, 10);

    let restored_dummy = snap_reconnect
        .iter()
        .find(|e| e.id == TRAINING_DUMMY_ID)
        .unwrap();
    assert_eq!(restored_dummy.hp, Some(75));

    tick_handle.abort();
}
