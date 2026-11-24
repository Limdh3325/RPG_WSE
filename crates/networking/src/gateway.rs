use crate::replicator::RegionReplicator;
use crate::session::{
    CommandRateLimiter, DEFAULT_MAX_QUEUE_SIZE, MAX_COMMANDS_PER_SECOND, Session,
};
use crate::ticket::{DEFAULT_TICKET_TTL, TicketStore};
use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use observability::WseMetricsRegistry;
use serde::{Deserialize, Serialize};
use shared_protocol::{
    ClientMessage, PROTOCOL_VERSION, PrologueRewardWire, ServerMessage, WireCommandPayload,
    WireCommandStatus, WsTicketRequest, WsTicketResponse,
};
use simulation_systems::prologue::{
    FilePrologueStorage, PrologueAction, PrologueManager, PrologueState,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_SHARED_SETTLEMENT_REGION_ID: Uuid =
    Uuid::from_u128(0x00000000_0000_0000_0000_000000000001);

#[derive(Debug, Clone)]
pub struct InboundCommand {
    pub session_id: Uuid,
    pub command_id: Uuid,
    pub actor_id: Uuid,
    pub region_id: Uuid,
    pub payload: WireCommandPayload,
}

use crate::simulation::SettlementSimulation;

#[derive(Clone)]
pub struct GatewayState {
    pub ticket_store: TicketStore,
    pub replicators: Arc<RwLock<HashMap<Uuid, Arc<RegionReplicator>>>>,
    pub command_sender: Option<mpsc::Sender<InboundCommand>>,
    pub prologue_manager: Arc<Mutex<PrologueManager>>,
    pub simulation: Option<Arc<SettlementSimulation>>,
    pub metrics: Arc<WseMetricsRegistry>,
}

impl GatewayState {
    pub fn new(command_sender: Option<mpsc::Sender<InboundCommand>>) -> Self {
        let prologue_dir =
            std::env::var("PROLOGUE_DATA_DIR").unwrap_or_else(|_| ".data/prologue".to_string());
        let storage = FilePrologueStorage::new(&prologue_dir).unwrap_or_else(|e| {
            panic!(
                "failed to initialize prologue storage at {}: {}",
                prologue_dir, e
            )
        });
        let prologue_manager = PrologueManager::try_new_with_storage(Arc::new(storage))
            .expect("failed to load prologue storage state");

        Self {
            ticket_store: TicketStore::new(),
            replicators: Arc::new(RwLock::new(HashMap::new())),
            command_sender,
            prologue_manager: Arc::new(Mutex::new(prologue_manager)),
            simulation: None,
            metrics: Arc::new(WseMetricsRegistry::global().clone()),
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<WseMetricsRegistry>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_simulation(mut self, simulation: Arc<SettlementSimulation>) -> Self {
        self.simulation = Some(simulation);
        self
    }

    pub fn with_settlement_simulation(mut self, region_id: Uuid) -> Self {
        let replicator = self.get_or_create_replicator(region_id);
        let sim = Arc::new(SettlementSimulation::new(region_id, replicator));
        self.simulation = Some(sim);
        self
    }

    pub fn with_prologue_manager(mut self, prologue_manager: Arc<Mutex<PrologueManager>>) -> Self {
        self.prologue_manager = prologue_manager;
        self
    }

    pub fn get_or_create_replicator(&self, region_id: Uuid) -> Arc<RegionReplicator> {
        let mut map = self
            .replicators
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.entry(region_id)
            .or_insert_with(|| Arc::new(RegionReplicator::new(region_id)))
            .clone()
    }
}

impl Default for GatewayState {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionStatusResponse {
    pub region_id: Uuid,
    pub epoch: u64,
    pub last_tick: u64,
    pub is_active: bool,
    pub subscriber_count: usize,
    pub entity_count: usize,
}

pub fn create_gateway_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/api/v1/ws-tickets", post(issue_ticket_handler))
        .route("/api/v1/regions/{id}/status", get(region_status_handler))
        .route("/metrics", get(metrics_handler))
        .route("/ws", get(ws_upgrade_handler))
        .with_state(state)
}

async fn metrics_handler(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    let body = state.metrics.render_prometheus_text();
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

async fn region_status_handler(
    State(state): State<Arc<GatewayState>>,
    Path(region_id): Path<Uuid>,
) -> Result<Json<RegionStatusResponse>, (StatusCode, String)> {
    let replicator = state.get_or_create_replicator(region_id);
    let subscriber_count = replicator.session_count();
    let epoch = 1u64;
    let last_tick = if let Some(sim) = &state.simulation {
        if sim.region_id == region_id {
            sim.current_tick()
        } else {
            replicator.world_revision()
        }
    } else {
        replicator.world_revision()
    };

    let entity_count = if let Some(sim) = &state.simulation {
        if sim.region_id == region_id {
            sim.entity_count()
        } else {
            0
        }
    } else {
        0
    };

    Ok(Json(RegionStatusResponse {
        region_id,
        epoch,
        last_tick,
        is_active: subscriber_count > 0 || entity_count > 0,
        subscriber_count,
        entity_count,
    }))
}

async fn issue_ticket_handler(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<WsTicketRequest>,
) -> Result<Json<WsTicketResponse>, (StatusCode, String)> {
    let (ticket, expires_in_secs) = state.ticket_store.issue_ticket(
        payload.account_id,
        payload.actor_id,
        payload.region_id,
        DEFAULT_TICKET_TTL,
    );

    info!(
        account_id = %payload.account_id,
        actor_id = %payload.actor_id,
        region_id = %payload.region_id,
        "Issued WS ticket"
    );

    Ok(Json(WsTicketResponse {
        ticket,
        expires_in_secs,
    }))
}

async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<GatewayState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // 1. Wait for AUTH message within timeout
    let auth_result = tokio::time::timeout(AUTH_TIMEOUT, async {
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(Message::Text(text)) => match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(ClientMessage::Auth { ticket }) => return Ok(ticket),
                    Ok(_) => {
                        return Err(("AUTH_REQUIRED", "First message must be AUTH".to_string()));
                    }
                    Err(e) => {
                        return Err(("BAD_REQUEST", format!("Invalid JSON message: {e}")));
                    }
                },
                Ok(Message::Ping(_)) => {}
                Ok(Message::Close(_)) => return Err(("CLOSED", "Client closed connection".into())),
                Err(e) => return Err(("IO_ERROR", format!("Socket read error: {e}"))),
                _ => {}
            }
        }
        Err(("CLOSED", "Stream terminated without auth".into()))
    })
    .await;

    let ticket = match auth_result {
        Ok(Ok(t)) => t,
        Ok(Err((code, msg))) => {
            let error_msg = ServerMessage::AuthError {
                code: code.to_string(),
                message: msg,
            };
            if let Ok(json) = serde_json::to_string(&error_msg) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            let _ = ws_sender.close().await;
            return;
        }
        Err(_) => {
            let error_msg = ServerMessage::AuthError {
                code: "AUTH_TIMEOUT".to_string(),
                message: "Authentication timed out after 5 seconds".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&error_msg) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            let _ = ws_sender.close().await;
            return;
        }
    };

    // 2. Consume single-use ticket
    let ticket_data = match state.ticket_store.consume_ticket(&ticket) {
        Some(data) => data,
        None => {
            let error_msg = ServerMessage::AuthError {
                code: "INVALID_TICKET".to_string(),
                message: "Ticket is invalid, expired, or already used".to_string(),
            };
            if let Ok(json) = serde_json::to_string(&error_msg) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            let _ = ws_sender.close().await;
            return;
        }
    };

    // Check if player has already completed the prologue.
    // If so, reconnects automatically bind to the shared settlement region rather than the private prologue instance.
    let effective_region_id = {
        let prologue = state
            .prologue_manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if prologue.is_completed(&ticket_data.actor_id) {
            let target = prologue
                .get_completed_target_region(&ticket_data.actor_id)
                .unwrap_or(DEFAULT_SHARED_SETTLEMENT_REGION_ID);
            info!(
                actor_id = %ticket_data.actor_id,
                ticket_region = %ticket_data.region_id,
                effective_region = %target,
                "Actor has completed prologue; routing session directly to shared settlement region"
            );
            target
        } else {
            ticket_data.region_id
        }
    };

    let session_id = Uuid::new_v4();
    let (server_tx, mut server_rx) = mpsc::channel::<ServerMessage>(DEFAULT_MAX_QUEUE_SIZE);

    let session = Arc::new(Session::new(
        session_id,
        ticket_data.account_id,
        ticket_data.actor_id,
        effective_region_id,
        server_tx.clone(),
    ));

    // Send AuthOk
    let auth_ok = ServerMessage::AuthOk {
        session_id,
        actor_id: ticket_data.actor_id,
        region_id: effective_region_id,
        protocol_version: PROTOCOL_VERSION,
    };
    if let Ok(json) = serde_json::to_string(&auth_ok) {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }

    let replicator = state.get_or_create_replicator(effective_region_id);
    replicator.add_session(session.clone());

    info!(
        %session_id,
        actor_id = %ticket_data.actor_id,
        region_id = %effective_region_id,
        "Client authenticated and session registered"
    );

    // Send initial prologue state sync to client
    // 3. Spawn outgoing message pump task
    let outgoing_task = tokio::spawn(async move {
        while let Some(server_msg) = server_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&server_msg) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
        }
        let _ = ws_sender.close().await;
    });

    if let Some(ref sim) = state.simulation {
        if sim.region_id == effective_region_id {
            sim.register_player(ticket_data.actor_id, None);
            let entities = sim.get_all_entities();
            let current_tick = sim.current_tick();
            let _ = replicator.send_initial_snapshot(&session_id, current_tick, &entities);
        }
    }

    // 4. Incoming message pump
    let mut rate_limiter = CommandRateLimiter::new(MAX_COMMANDS_PER_SECOND);
    let session_for_cleanup = session.clone();
    let replicator_for_cleanup = replicator.clone();

    while let Some(msg_res) = ws_receiver.next().await {
        let text = match msg_res {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(_payload)) => {
                // Axum handles ping/pong or we can ignore
                continue;
            }
            Ok(Message::Close(_)) => break,
            Err(_) => break,
            _ => continue,
        };

        let client_msg = match serde_json::from_str::<ClientMessage>(&text) {
            Ok(m) => m,
            Err(e) => {
                warn!(%session_id, ?e, "Failed to parse client message");
                continue;
            }
        };

        match client_msg {
            ClientMessage::Ping { nonce } => {
                let _ = session.send_nonblocking(ServerMessage::Pong { nonce });
            }
            ClientMessage::SnapshotAck { world_revision, .. } => {
                session.update_revision(world_revision);
            }
            ClientMessage::DeltaAck { delta_seq } => {
                debug!(%session_id, delta_seq, "Delta acknowledged");
            }
            ClientMessage::ResyncRequest { reason, .. } => {
                debug!(%session_id, %reason, "Client requested resync");
                state.metrics.record_client_resync(&reason);
                if let Some(ref sim) = state.simulation {
                    let entities = sim.get_all_entities();
                    let current_tick = sim.current_tick();
                    let _ = replicator.handle_resync(&session_id, current_tick, &entities);
                } else {
                    let _ = replicator.handle_resync(&session_id, 0, &[]);
                }
            }
            ClientMessage::Command {
                command_id,
                actor_id,
                payload,
            } => {
                let region_str = session.region_id().to_string();
                // Rate limiter check
                if !rate_limiter.check_and_record() {
                    state
                        .metrics
                        .record_command_rejection(&region_str, "rate_limited");
                    let _ = session.send_nonblocking(ServerMessage::CommandStatus {
                        command_id,
                        status: WireCommandStatus::Rejected,
                        reason: Some("Rate limit exceeded".into()),
                        committed_event_seq: None,
                    });
                    continue;
                }

                // Security check: actor_id verification (TEST-NET-004)
                if actor_id != session.actor_id {
                    state
                        .metrics
                        .record_command_rejection(&region_str, "auth_mismatch");
                    warn!(
                        %session_id,
                        session_actor = %session.actor_id,
                        claimed_actor = %actor_id,
                        "Forged actor ID detected; rejecting command"
                    );
                    let _ = session.send_nonblocking(ServerMessage::CommandStatus {
                        command_id,
                        status: WireCommandStatus::Rejected,
                        reason: Some("Actor ID does not match authenticated session".into()),
                        committed_event_seq: None,
                    });
                    continue;
                }

                // Dispatch to engine simulation or command channel if configured
                if let Some(ref sim) = state.simulation {
                    let cmd = InboundCommand {
                        session_id,
                        command_id,
                        actor_id,
                        region_id: session.region_id(),
                        payload,
                    };
                    let status_msg = sim.apply_command(&session, cmd);
                    let _ = session.send_nonblocking(status_msg);
                } else if let Some(ref tx) = state.command_sender {
                    let cmd = InboundCommand {
                        session_id,
                        command_id,
                        actor_id,
                        region_id: session.region_id(),
                        payload,
                    };
                    if let Err(e) = tx.try_send(cmd) {
                        state
                            .metrics
                            .record_command_rejection(&region_str, "queue_full");
                        warn!(%session_id, ?e, "Inbound command queue full or dropped");
                        let _ = session.send_nonblocking(ServerMessage::CommandStatus {
                            command_id,
                            status: WireCommandStatus::Rejected,
                            reason: Some("Server command queue full".into()),
                            committed_event_seq: None,
                        });
                    }
                } else {
                    // Default mock accept for standalone testing
                    let _ = session.send_nonblocking(ServerMessage::CommandStatus {
                        command_id,
                        status: WireCommandStatus::Accepted,
                        reason: None,
                        committed_event_seq: None,
                    });
                }
            }
            ClientMessage::Auth { .. } => {
                // Redundant auth ignored
            }
            ClientMessage::PrologueQueryState => {
                let manager = state
                    .prologue_manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let sync = make_prologue_sync_message(&manager, &session.actor_id);
                let _ = session.send_nonblocking(sync);
            }
            ClientMessage::PrologueStart => {
                let mut manager = state
                    .prologue_manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let private_region = Uuid::new_v4();
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = manager.start_prologue(session.actor_id, private_region, now_ms) {
                    warn!(%session_id, ?e, "Failed to start prologue for actor");
                }
                let sync = make_prologue_sync_message(&manager, &session.actor_id);
                let _ = session.send_nonblocking(sync);
            }
            ClientMessage::PrologueStepAction {
                action_type,
                target_id,
                payload: _,
            } => {
                let mut manager = state
                    .prologue_manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let action = match action_type.as_str() {
                    "talk" | "talk_to_npc" => PrologueAction::TalkToNpc {
                        npc_name: target_id.unwrap_or_else(|| "Elder Elda".to_string()),
                    },
                    "gather" | "gather_resource" => PrologueAction::GatherResource {
                        resource_type: target_id.unwrap_or_else(|| "wild_herbs".to_string()),
                        amount: 1,
                    },
                    "waypoint" | "reach_waypoint" => {
                        PrologueAction::ReachWaypoint { x: 100, y: 100 }
                    }
                    "defeat" | "defeat_encounter" => PrologueAction::DefeatEncounter {
                        target_id: target_id.unwrap_or_else(|| "goblin_scout".to_string()),
                    },
                    "pack" | "pack_supplies" => PrologueAction::PackSupplies {
                        supply_id: target_id
                            .unwrap_or_else(|| "frontier_cart_supplies".to_string()),
                    },
                    "enter_gate" | "enter_settlement" => PrologueAction::EnterSettlementGate,
                    _ => PrologueAction::EnterSettlementGate,
                };
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = manager.advance_step(session.actor_id, action, now_ms) {
                    warn!(%session_id, ?e, "Failed to advance prologue step");
                }
                let sync = make_prologue_sync_message(&manager, &session.actor_id);
                let _ = session.send_nonblocking(sync);
            }
            ClientMessage::PrologueFallback { reason } => {
                let mut manager = state
                    .prologue_manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let reason_lower = reason.to_lowercase();
                if reason_lower.contains("casualty")
                    || reason_lower.contains("combat")
                    || reason_lower.contains("death")
                    || reason_lower.contains("defeat")
                    || reason_lower.contains("advance")
                {
                    if let Err(e) =
                        manager.trigger_npc_casualty_fallback(session.actor_id, &reason, now_ms)
                    {
                        warn!(%session_id, ?e, "Failed to trigger prologue combat/casualty fallback");
                    }
                } else if let Err(e) =
                    manager.report_movement_failure(session.actor_id, &reason, now_ms)
                {
                    warn!(%session_id, ?e, "Failed to report prologue movement failure");
                }
                let sync = make_prologue_sync_message(&manager, &session.actor_id);
                let _ = session.send_nonblocking(sync);
            }
            ClientMessage::PrologueRequestTransfer { target_region_id } => {
                let mut manager = state
                    .prologue_manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                // Enforce authoritative shared settlement region (prevent arbitrary client destination hijacking)
                let target = target_region_id
                    .filter(|&id| id == DEFAULT_SHARED_SETTLEMENT_REGION_ID)
                    .unwrap_or(DEFAULT_SHARED_SETTLEMENT_REGION_ID);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let exec = manager.transfer_to_shared_world(session.actor_id, target, now_ms);
                match exec {
                    Ok(record) => {
                        let rewards_wire = record
                            .rewards
                            .iter()
                            .map(|r| PrologueRewardWire {
                                receipt_id: r.receipt_id,
                                sku: r.sku.clone(),
                                quantity: r.quantity,
                                granted_at_ms: r.granted_at_ms,
                            })
                            .collect();
                        let _ = session.send_nonblocking(ServerMessage::PrologueTransferResult {
                            transfer_id: record.transfer_id,
                            status: "COMMITTED".to_string(),
                            target_region_id: target,
                            rewards: rewards_wire,
                        });

                        // Replicate live session transfer to the target shared world region
                        let current_region = session.region_id();
                        if current_region != target {
                            {
                                let reps = state
                                    .replicators
                                    .read()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                                if let Some(old_rep) = reps.get(&current_region) {
                                    old_rep.remove_session(&session.session_id);
                                }
                            }
                            session.set_region_id(target);
                            let new_rep = state.get_or_create_replicator(target);
                            new_rep.add_session(session.clone());
                            let _ = new_rep.send_initial_snapshot(&session.session_id, 1, &[]);
                            info!(
                                %session_id,
                                from_region = %current_region,
                                to_region = %target,
                                "Migrated session to shared world replicator and sent baseline snapshot"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(%session_id, ?e, "Failed to complete prologue transfer");
                    }
                }
                let sync = make_prologue_sync_message(&manager, &session.actor_id);
                let _ = session.send_nonblocking(sync);
            }
        }
    }

    // 5. Cleanup
    let final_region = session_for_cleanup.region_id();
    let reps = state
        .replicators
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(rep) = reps.get(&final_region) {
        rep.remove_session(&session_for_cleanup.session_id);
    } else {
        replicator_for_cleanup.remove_session(&session_for_cleanup.session_id);
    }
    if let Some(ref sim) = state.simulation {
        sim.unregister_player(&session_for_cleanup.actor_id);
    }
    outgoing_task.abort();
    info!(%session_id, "Session disconnected and cleaned up");
}

fn make_prologue_sync_message(manager: &PrologueManager, actor_id: &Uuid) -> ServerMessage {
    let is_completed = manager.is_completed(actor_id);
    let state = manager.get_state(actor_id);
    match state {
        PrologueState::NotStarted => ServerMessage::PrologueStateSync {
            state: "NOT_STARTED".to_string(),
            step: "NONE".to_string(),
            progress: 0,
            target_count: 0,
            step_title: "Prologue Not Started".to_string(),
            step_description: "Begin your journey into the frontier.".to_string(),
            fallback_active: false,
            last_failure_reason: None,
            is_completed,
        },
        PrologueState::InProgress {
            step,
            progress,
            target_count,
            fallback_active,
            last_failure_reason,
            ..
        } => ServerMessage::PrologueStateSync {
            state: "IN_PROGRESS".to_string(),
            step: format!("{:?}", step),
            progress,
            target_count,
            step_title: format!("{:?}", step),
            step_description: step.description().to_string(),
            fallback_active,
            last_failure_reason,
            is_completed,
        },
        PrologueState::ReadyToTransfer { .. } => ServerMessage::PrologueStateSync {
            state: "READY_TO_TRANSFER".to_string(),
            step: "P5_SHARED_SETTLEMENT".to_string(),
            progress: 1,
            target_count: 1,
            step_title: "Ready For Settlement Gate".to_string(),
            step_description: "Ready to transfer into shared frontier settlement.".to_string(),
            fallback_active: false,
            last_failure_reason: None,
            is_completed,
        },
        PrologueState::Transferring { .. } => ServerMessage::PrologueStateSync {
            state: "TRANSFERRING".to_string(),
            step: "P5_SHARED_SETTLEMENT".to_string(),
            progress: 1,
            target_count: 1,
            step_title: "Transferring World State".to_string(),
            step_description: "Migrating character and items to shared world...".to_string(),
            fallback_active: false,
            last_failure_reason: None,
            is_completed,
        },
        PrologueState::Completed { .. } => ServerMessage::PrologueStateSync {
            state: "COMPLETED".to_string(),
            step: "COMPLETED".to_string(),
            progress: 1,
            target_count: 1,
            step_title: "Prologue Completed".to_string(),
            step_description: "You have arrived at the shared settlement.".to_string(),
            fallback_active: false,
            last_failure_reason: None,
            is_completed: true,
        },
    }
}
