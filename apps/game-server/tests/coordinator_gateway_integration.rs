use futures_util::{SinkExt, StreamExt};
use game_domain::id::{RegionId, SessionId};
use networking::gateway::{GatewayState, create_gateway_router};
use shared_protocol::{ClientMessage, ServerMessage, WireCommandPayload, WireCommandStatus};
use simulation_core::coordinator::RegionCoordinator;
use simulation_systems::economy::SettlementEconomy;
use simulation_systems::faction::FactionManager;
use simulation_systems::integrated_hooks::IntegratedSimulationHooks;
use simulation_systems::rule_dsl::DynamicRuleRegistry;
use simulation_systems::scheduler::NpcScheduler;
use simulation_systems::system::NpcAiSystem;
use sqlx::types::Uuid;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::test]
async fn test_game_server_coordinator_and_websocket_command_routing() {
    let coordinator = Arc::new(RegionCoordinator::new());
    let region_id_uuid = Uuid::new_v4();
    let region_id = RegionId(region_id_uuid);
    let epoch = 1;
    let seed = 4242;
    let tick_interval = Duration::from_millis(50);
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(100);

    let hooks = Box::new(IntegratedSimulationHooks::new(
        NpcAiSystem::default(),
        NpcScheduler::default(),
        SettlementEconomy::new_default(1),
        FactionManager::default(),
        DynamicRuleRegistry::default(),
        SessionId::from_u128(seed as u128),
        region_id,
    ));

    let _handle = coordinator
        .spawn_region_configured(
            region_id,
            epoch,
            ecs_components::grid::MapGrid::default_64x64(),
            seed,
            Some(event_tx),
            Some(hooks),
            Some(tick_interval),
        )
        .expect("spawn region configured");

    let state = Arc::new(GatewayState::default().with_coordinator(coordinator));

    // Connect background tick output listener
    tokio::spawn(async move {
        while let Some(_output) = event_rx.recv().await {
            // drained
        }
    });

    let app = create_gateway_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    // 1. Issue Ticket
    let actor_id = sqlx::types::Uuid::new_v4();
    let (ticket, _) = state.ticket_store.issue_ticket(
        sqlx::types::Uuid::new_v4(),
        actor_id,
        region_id_uuid,
        Duration::from_secs(60),
    );

    // 2. Connect WebSocket
    let ws_url = format!("ws://127.0.0.1:{}/ws", addr.port());
    let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect failed");

    // 3. Send Auth
    let auth_msg = ClientMessage::Auth { ticket };
    ws.send(Message::Text(serde_json::to_string(&auth_msg).unwrap().into()))
        .await
        .unwrap();

    // 4. Expect AuthOk
    let msg = ws.next().await.unwrap().unwrap();
    if let Message::Text(txt) = msg {
        let s_msg: ServerMessage = serde_json::from_str(&txt).unwrap();
        match s_msg {
            ServerMessage::AuthOk { actor_id: a_id, .. } => {
                assert_eq!(a_id, actor_id);
            }
            other => panic!("expected AuthOk, got: {:?}", other),
        }
    }

    // 5. Send Command to RegionCoordinator via Gateway
    let command_id = sqlx::types::Uuid::new_v4();
    let cmd = ClientMessage::Command {
        command_id,
        actor_id,
        payload: WireCommandPayload::Move {
            target_x: 32_000,
            target_y: 32_000,
        },
    };
    ws.send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
        .await
        .unwrap();

    // 6. Receive CommandStatus Accepted from RegionCoordinator
    let reply = ws.next().await.unwrap().unwrap();
    if let Message::Text(txt) = reply {
        let s_msg: ServerMessage = serde_json::from_str(&txt).unwrap();
        match s_msg {
            ServerMessage::CommandStatus {
                command_id: c_id,
                status,
                reason,
                ..
            } => {
                assert_eq!(c_id, command_id);
                assert_eq!(status, WireCommandStatus::Accepted);
                assert!(reason.is_none());
            }
            other => panic!("expected CommandStatus, got: {:?}", other),
        }
    }
}
