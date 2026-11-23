use std::sync::Arc;
use std::time::Duration;

use networking::gateway::{AdminDrainResponse, GatewayState, create_gateway_router};
use shared_protocol::{WsTicketRequest, WsTicketResponse};
use simulation_systems::prologue::{
    FilePrologueStorage, PrologueAction, PrologueManager, PrologueState, PrologueStep,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

async fn setup_operations_server(
    admin_token: Option<&str>,
    custom_data_dir: Option<std::path::PathBuf>,
) -> (String, std::net::SocketAddr, Arc<GatewayState>) {
    let state = match custom_data_dir {
        Some(dir) => {
            std::fs::create_dir_all(&dir).unwrap();
            let storage = FilePrologueStorage::new(&dir).unwrap();
            let manager = PrologueManager::try_new_with_storage(Arc::new(storage)).unwrap();
            let mut s = GatewayState::new(None)
                .with_prologue_manager(Arc::new(std::sync::Mutex::new(manager)));
            if let Some(tok) = admin_token {
                s = s.with_admin_token(tok);
            }
            Arc::new(s)
        }
        None => {
            let mut s = GatewayState::new(None);
            if let Some(tok) = admin_token {
                s = s.with_admin_token(tok);
            }
            Arc::new(s)
        }
    };

    let app = create_gateway_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    (format!("http://127.0.0.1:{}", addr.port()), addr, state)
}

async fn http_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body_str = body.unwrap_or("");
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        method,
        path,
        addr,
        body_str.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    req.push_str("\r\n");
    req.push_str(body_str);

    stream.write_all(req.as_bytes()).await.unwrap();
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

#[tokio::test]
async fn test_m8_001_health_live_and_ready_probes() {
    let (_url, addr, _state) = setup_operations_server(None, None).await;

    // Liveness probe
    let (status, _headers, body) = http_request(addr, "GET", "/health/live", &[], None).await;
    assert_eq!(status, 200, "/health/live must return 200 OK");
    assert_eq!(body, "OK");

    // Readiness probe
    let (status, _headers, body) = http_request(addr, "GET", "/health/ready", &[], None).await;
    assert_eq!(status, 200, "/health/ready must return 200 OK");
    assert_eq!(body, "READY");
}

#[tokio::test]
async fn test_m8_002_admin_rbac_authorization() {
    let secret_token = "secret-production-admin-key-998877";
    let (_url, addr, _state) = setup_operations_server(Some(secret_token), None).await;

    // 1. Missing Authorization header -> 401 Unauthorized
    let (status, _, _) = http_request(addr, "POST", "/api/v1/admin/drain", &[], Some("{}")).await;
    assert_eq!(status, 401, "Missing token must be rejected with 401");

    // 2. Invalid Bearer token -> 403 Forbidden
    let bad_auth = [("Authorization", "Bearer invalid-wrong-token")];
    let (status, _, _) =
        http_request(addr, "POST", "/api/v1/admin/drain", &bad_auth, Some("{}")).await;
    assert_eq!(status, 403, "Invalid token must be rejected with 403");

    // 3. Valid Bearer token -> 200 OK + AdminDrainResponse JSON
    let valid_auth = [("Authorization", "Bearer secret-production-admin-key-998877")];
    let (status, _, body) =
        http_request(addr, "POST", "/api/v1/admin/drain", &valid_auth, Some("{}")).await;
    assert_eq!(status, 200, "Valid token must succeed with 200 OK");
    let resp: AdminDrainResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(resp.status, "DRAINING");
}

#[tokio::test]
async fn test_m8_003_graceful_drain_lifecycle() {
    let secret_token = "admin-drain-key";
    let (_url, addr, state) = setup_operations_server(Some(secret_token), None).await;

    // Before drain: readiness 200 OK, ticket issuance succeeds
    let (status, _, _) = http_request(addr, "GET", "/health/ready", &[], None).await;
    assert_eq!(status, 200);

    let ticket_req = serde_json::to_string(&WsTicketRequest {
        account_id: Uuid::new_v4(),
        actor_id: Uuid::new_v4(),
        region_id: Uuid::new_v4(),
    })
    .unwrap();
    let (status, _, body) = http_request(
        addr,
        "POST",
        "/api/v1/ws-tickets",
        &[("Content-Type", "application/json")],
        Some(&ticket_req),
    )
    .await;
    assert_eq!(status, 200);
    let _t_res: WsTicketResponse = serde_json::from_str(&body).unwrap();

    // Trigger Graceful Drain via Admin API
    let auth = [("Authorization", "Bearer admin-drain-key")];
    let (status, _, _) = http_request(addr, "POST", "/api/v1/admin/drain", &auth, Some("{}")).await;
    assert_eq!(status, 200);
    assert!(state.is_draining());

    // After drain:
    // 1. /health/live remains 200 OK
    let (status, _, body) = http_request(addr, "GET", "/health/live", &[], None).await;
    assert_eq!(status, 200);
    assert_eq!(body, "OK");

    // 2. /health/ready returns 503 Service Unavailable ("DRAINING")
    let (status, _, body) = http_request(addr, "GET", "/health/ready", &[], None).await;
    assert_eq!(status, 503, "Readiness must return 503 during drain");
    assert_eq!(body, "DRAINING");

    // 3. New ticket issuance is rejected with 503
    let (status, _, _) = http_request(
        addr,
        "POST",
        "/api/v1/ws-tickets",
        &[("Content-Type", "application/json")],
        Some(&ticket_req),
    )
    .await;
    assert_eq!(
        status, 503,
        "Ticket issuance must be rejected with 503 during drain"
    );
}

#[tokio::test]
async fn test_m8_004_prologue_durability_and_restart_recovery() {
    let temp_dir = std::env::temp_dir().join(format!("wse_test_m8_storage_{}", Uuid::new_v4()));
    let _account_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let region_id = Uuid::new_v4();

    // Instance 1: write prologue session and advance
    let _t0 = std::time::Instant::now();
    {
        let (_url, _addr, state) = setup_operations_server(None, Some(temp_dir.clone())).await;
        let mut mgr = state.prologue_manager.lock().unwrap();
        mgr.start_prologue(actor_id, region_id, 100).unwrap();
        mgr.advance_step(
            actor_id,
            PrologueAction::TalkToNpc {
                npc_name: "elder_elda".into(),
            },
            105,
        )
        .unwrap();

        let current_state = mgr.get_state(&actor_id);
        assert!(matches!(
            current_state,
            PrologueState::InProgress {
                step: PrologueStep::P1ResourceGathering,
                ..
            }
        ));
    }

    // Instance 2: Cold reboot server with identical storage (measuring RTO)
    let t_recovery_start = std::time::Instant::now();
    {
        let (_url, addr, state) = setup_operations_server(None, Some(temp_dir.clone())).await;
        let mgr = state.prologue_manager.lock().unwrap();
        let current_state = mgr.get_state(&actor_id);
        assert!(
            matches!(
                current_state,
                PrologueState::InProgress {
                    step: PrologueStep::P1ResourceGathering,
                    ..
                }
            ),
            "Session state must survive server reboot without data loss"
        );

        // Verify recovered server readiness
        let (status, _, body) = http_request(addr, "GET", "/health/ready", &[], None).await;
        assert_eq!(status, 200);
        assert_eq!(body, "READY");
    }
    let rto_elapsed = t_recovery_start.elapsed();
    println!(
        "Reboot and recovery RTO measured: {:?} (target <= 60s)",
        rto_elapsed
    );
    assert!(
        rto_elapsed < Duration::from_secs(5),
        "RTO must be well within operational bounds (<5s in local drill)"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
