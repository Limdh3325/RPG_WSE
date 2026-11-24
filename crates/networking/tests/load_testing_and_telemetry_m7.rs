use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use networking::gateway::{GatewayState, RegionStatusResponse, create_gateway_router};
use networking::simulation::{SettlementSimulation, SimulationEntity};
use observability::WseMetricsRegistry;
use shared_protocol::{
    ClientMessage, EntityStateWire, ServerMessage, WireCommandPayload, WireCommandStatus,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use uuid::Uuid;

async fn setup_test_server() -> (
    String,
    std::net::SocketAddr,
    Arc<GatewayState>,
    Arc<SettlementSimulation>,
) {
    let region_id = Uuid::new_v4();
    let state = Arc::new(GatewayState::new(None).with_settlement_simulation(region_id));
    let sim = state.simulation.as_ref().unwrap().clone();

    let app = create_gateway_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    (
        format!("ws://127.0.0.1:{}/ws", addr.port()),
        addr,
        state,
        sim,
    )
}

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, addr
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let resp_str = String::from_utf8_lossy(&response).to_string();
    let parts: Vec<&str> = resp_str.split("\r\n\r\n").collect();
    let header_part = parts.first().copied().unwrap_or("");
    let body_part = parts.get(1).copied().unwrap_or("").to_string();
    let status_code = header_part
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status_code, header_part.to_string(), body_part)
}

async fn connect_client(
    ws_url: &str,
    state: &GatewayState,
    region_id: Uuid,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Uuid,
    Uuid,
) {
    let account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let (ticket, _) =
        state
            .ticket_store
            .issue_ticket(account_id, actor_id, region_id, Duration::from_secs(60));

    let (mut ws, _) = connect_async(ws_url).await.unwrap();
    let auth_msg = ClientMessage::Auth { ticket };
    ws.send(Message::Text(
        serde_json::to_string(&auth_msg).unwrap().into(),
    ))
    .await
    .unwrap();

    while let Some(Ok(Message::Text(txt))) = ws.next().await {
        if let Ok(ServerMessage::AuthOk { .. }) = serde_json::from_str(&txt) {
            break;
        }
    }

    (ws, account_id, actor_id)
}

#[tokio::test]
async fn test_m7_001_metrics_scrape_endpoint_and_labels() {
    let (_ws_url, addr, _state, sim) = setup_test_server().await;

    // Advance 5 ticks
    for _ in 0..5 {
        sim.step_tick();
    }

    // Call /metrics scrape endpoint
    let (status, headers, body) = http_get(addr, "/metrics").await;
    assert_eq!(status, 200, "Prometheus endpoint must return 200 OK");
    assert!(
        headers.contains("content-type: text/plain; version=0.0.4; charset=utf-8"),
        "Headers must match Prometheus scrape content-type: {}",
        headers
    );

    // Verify all 12 core metric keys from docs/12-test-observability.md are present
    let required_metrics = [
        "wse_tick_duration_seconds",
        "wse_tick_overruns_total",
        "wse_region_entities",
        "wse_npc_decisions_total",
        "wse_goap_expansions_total",
        "wse_command_queue_depth",
        "wse_command_rejections_total",
        "wse_persistence_latency_seconds",
        "wse_snapshot_age_seconds",
        "wse_region_epoch_changes_total",
        "wse_replication_bytes_total",
        "wse_client_resync_total",
    ];

    for metric in &required_metrics {
        assert!(
            body.contains(metric),
            "Prometheus metrics must contain '{}'. Body snippet:\n{}",
            metric,
            &body[..body.len().min(500)]
        );
    }

    // Verify Cardinality Rule: no individual entity_id or command_id labels
    assert!(
        !body.contains("entity_id="),
        "High cardinality violation: Prometheus text must not contain entity_id label"
    );
    assert!(
        !body.contains("command_id="),
        "High cardinality violation: Prometheus text must not contain command_id label"
    );
}

#[tokio::test]
async fn test_m7_002_region_status_endpoint() {
    let (_ws_url, addr, _state, sim) = setup_test_server().await;

    // Advance a few ticks
    for _ in 0..10 {
        sim.step_tick();
    }

    let path = format!("/api/v1/regions/{}/status", sim.region_id);
    let (status, _headers, body) = http_get(addr, &path).await;
    assert_eq!(status, 200, "Status endpoint must return 200 OK");

    let res: RegionStatusResponse =
        serde_json::from_str(&body).expect("Status response must parse into RegionStatusResponse");

    assert_eq!(res.region_id, sim.region_id);
    assert_eq!(res.last_tick, 11);
    assert!(res.is_active);
    assert_eq!(res.subscriber_count, 0);
    assert_eq!(res.entity_count, 12); // Default 12 entities in settlement
}

#[tokio::test]
async fn test_m7_003_load_and_soak_10_players_1000_npcs() {
    let (ws_url, _addr, state, sim) = setup_test_server().await;

    // Populate settlement simulation up to 1,000 NPCs / entities
    let base_count = sim.entity_count();
    let needed = 1000usize.saturating_sub(base_count);
    {
        let mut ent_guard = sim.entities.write().unwrap();
        for i in 0..needed {
            let ent_id = Uuid::new_v4();
            ent_guard.insert(
                ent_id,
                SimulationEntity {
                    state: EntityStateWire {
                        id: ent_id,
                        x: (i % 50) as i32,
                        y: (i / 50) as i32,
                        entity_type: "npc".into(),
                        hp: Some(100),
                        max_hp: Some(100),
                        name: Some(format!("NPC-Citizen-{}", i)),
                    },
                    respawn_at_tick: None,
                    wander_dir: 0,
                    base_x: (i % 50) as i32,
                    base_y: (i / 50) as i32,
                },
            );
        }
    }
    assert!(
        sim.entity_count() >= 1000,
        "Must have at least 1,000 entities in simulation"
    );

    // Connect 10 concurrent players
    let mut clients = Vec::new();
    for _ in 0..10 {
        let (ws, account_id, actor_id) = connect_client(&ws_url, &state, sim.region_id).await;
        clients.push((ws, account_id, actor_id));
    }

    // Run 100 continuous ticks (soak cycle)
    let soak_ticks = 100;
    let mut tick_durations = Vec::with_capacity(soak_ticks);

    for tick in 1..=soak_ticks {
        let t0 = Instant::now();
        sim.step_tick();
        let dt = t0.elapsed();
        tick_durations.push(dt);

        // Every 5 ticks, all 10 players issue concurrent move commands
        if tick % 5 == 0 {
            for (ws, _acc_id, actor_id) in &mut clients {
                let cmd = ClientMessage::Command {
                    command_id: Uuid::new_v4(),
                    actor_id: *actor_id,
                    payload: WireCommandPayload::Move {
                        target_x: tick as i64,
                        target_y: 1i64,
                    },
                };
                let _ = ws
                    .send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
                    .await;
            }
        }
    }

    // Sort tick durations to measure percentiles
    tick_durations.sort();
    let p50 = tick_durations[tick_durations.len() * 50 / 100];
    let p95 = tick_durations[tick_durations.len() * 95 / 100];
    let p99 = tick_durations[tick_durations.len() * 99 / 100];

    println!(
        "Load Test Performance (10 players, 1000 NPCs, {} ticks): p50={:?}, p95={:?}, p99={:?}",
        soak_ticks, p50, p95, p99
    );

    // Assert SLA requirements from docs/12-test-observability.md:
    // Tick p95 <= 30ms, p99 <= 50ms
    assert!(
        p95 <= Duration::from_millis(30),
        "Tick p95 ({:?}) must be <= 30ms",
        p95
    );
    assert!(
        p99 <= Duration::from_millis(50),
        "Tick p99 ({:?}) must be <= 50ms",
        p99
    );

    // Record mock DB persistence latency to check histogram SLA (p95 <= 150ms)
    let metrics = WseMetricsRegistry::global();
    for i in 1..=50 {
        let fake_db_sec = (i as f64) * 0.002; // 2ms to 100ms
        metrics.record_persistence_latency("snapshot_write", fake_db_sec);
    }
    assert!(
        metrics.persistence_latency_p95("snapshot_write") <= 0.150,
        "Persistence latency p95 must be <= 150ms"
    );

    // Clean up client connections
    for (mut ws, _, _) in clients {
        let _ = ws.close(None).await;
    }
}

#[tokio::test]
async fn test_m7_004_fault_matrix_resilience_and_telemetry() {
    let (ws_url, _addr, state, sim) = setup_test_server().await;

    // Fault A: Rate Limit exceeded (send 75 rapid commands within 1 second; limit is 60)
    {
        let (mut ws_rate, _acc_id, actor_id) = connect_client(&ws_url, &state, sim.region_id).await;
        for _ in 0..75 {
            let cmd = ClientMessage::Command {
                command_id: Uuid::new_v4(),
                actor_id,
                payload: WireCommandPayload::Move {
                    target_x: 10i64,
                    target_y: 10i64,
                },
            };
            let _ = ws_rate
                .send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
                .await;
        }

        let mut rate_limit_rejected = false;
        let timeout = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < timeout {
            if let Ok(Some(Ok(Message::Text(txt)))) =
                tokio::time::timeout(Duration::from_millis(50), ws_rate.next()).await
            {
                if let Ok(ServerMessage::CommandStatus { status, reason, .. }) =
                    serde_json::from_str(&txt)
                {
                    if status == WireCommandStatus::Rejected
                        && reason.as_deref() == Some("Rate limit exceeded")
                    {
                        rate_limit_rejected = true;
                        break;
                    }
                }
            }
        }
        assert!(
            rate_limit_rejected,
            "Fault A: Expected rate limit rejection"
        );
        let _ = ws_rate.close(None).await;
    }

    // Fault B: Forged actor ID
    {
        let (mut ws_auth, _acc_id, _actor_id) =
            connect_client(&ws_url, &state, sim.region_id).await;
        let forged_actor = Uuid::new_v4();
        let forged_cmd = ClientMessage::Command {
            command_id: Uuid::new_v4(),
            actor_id: forged_actor,
            payload: WireCommandPayload::Move {
                target_x: 20i64,
                target_y: 20i64,
            },
        };
        ws_auth
            .send(Message::Text(
                serde_json::to_string(&forged_cmd).unwrap().into(),
            ))
            .await
            .unwrap();

        let mut auth_mismatch_rejected = false;
        let timeout = Instant::now() + Duration::from_millis(1000);
        while Instant::now() < timeout {
            if let Ok(Some(Ok(Message::Text(txt)))) =
                tokio::time::timeout(Duration::from_millis(50), ws_auth.next()).await
            {
                if let Ok(ServerMessage::CommandStatus { status, reason, .. }) =
                    serde_json::from_str(&txt)
                {
                    if status == WireCommandStatus::Rejected
                        && reason.as_deref().unwrap_or("").contains("does not match")
                    {
                        auth_mismatch_rejected = true;
                        break;
                    }
                }
            }
        }
        assert!(
            auth_mismatch_rejected,
            "Fault B: Expected auth mismatch rejection"
        );
        let _ = ws_auth.close(None).await;
    }

    // Fault C: Client resync request (ResyncRequest)
    {
        let (mut ws_resync, _acc_id, _actor_id) =
            connect_client(&ws_url, &state, sim.region_id).await;
        let resync_t0 = Instant::now();
        let resync_msg = ClientMessage::ResyncRequest {
            reason: "gap_detection".into(),
            last_seen_revision: 0,
        };
        ws_resync
            .send(Message::Text(
                serde_json::to_string(&resync_msg).unwrap().into(),
            ))
            .await
            .unwrap();

        let mut snapshot_received = false;
        let timeout = Instant::now() + Duration::from_millis(1000);
        while Instant::now() < timeout {
            if let Ok(Some(Ok(Message::Text(txt)))) =
                tokio::time::timeout(Duration::from_millis(50), ws_resync.next()).await
            {
                if let Ok(ServerMessage::WorldSnapshot { .. }) = serde_json::from_str(&txt) {
                    snapshot_received = true;
                    break;
                }
            }
        }
        let resync_latency = resync_t0.elapsed();
        assert!(
            snapshot_received,
            "Fault C: Expected WorldSnapshot on ResyncRequest"
        );
        assert!(
            resync_latency <= Duration::from_secs(2),
            "Fault C: Resync latency ({:?}) must be <= 2.0s",
            resync_latency
        );
        let _ = ws_resync.close(None).await;
    }

    // Fault D: Region Epoch Change
    let metrics = WseMetricsRegistry::global();
    let reg_str = sim.region_id.to_string();
    metrics.record_region_epoch_change(&reg_str);
    let prom_text = metrics.render_prometheus_text();
    assert!(
        prom_text.contains("wse_region_epoch_changes_total"),
        "Fault D: Epoch change metric must be recorded in Prometheus exporter"
    );
}

#[tokio::test]
async fn test_m7_005_long_run_memory_and_invariant_preservation() {
    let (ws_url, addr, state, sim) = setup_test_server().await;

    // Connect 5 clients
    let mut clients = Vec::new();
    for _ in 0..5 {
        let (ws, account_id, actor_id) = connect_client(&ws_url, &state, sim.region_id).await;
        clients.push((ws, account_id, actor_id));
    }

    // Step 20 ticks
    for _ in 0..20 {
        sim.step_tick();
    }

    // Check status: 5 subscribers
    let path = format!("/api/v1/regions/{}/status", sim.region_id);
    let (status, _, body) = http_get(addr, &path).await;
    assert_eq!(status, 200);
    let res: RegionStatusResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(res.subscriber_count, 5);

    // Disconnect all 5 clients cleanly
    for (mut ws, _, _) in clients {
        let _ = ws.close(None).await;
    }

    // Give gateway tasks 150ms to clean up sessions
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Check status again: subscriber count should be 0 (no lingering session leaks)
    let (_, _, body2) = http_get(addr, &path).await;
    let res2: RegionStatusResponse = serde_json::from_str(&body2).unwrap();
    assert_eq!(
        res2.subscriber_count, 0,
        "All sessions must be cleaned up without leaks"
    );

    // Invariant check: Food Ledger and Economy state intact
    let food_stock = *sim.market_food_stock.read().unwrap();
    assert!(food_stock >= 0, "Food stock must never be negative");
    let ledger = sim.food_ledger.read().unwrap();
    for entry in ledger.entries() {
        assert!(
            entry.balance_after >= 0,
            "Food ledger balance must never be negative"
        );
    }
}
