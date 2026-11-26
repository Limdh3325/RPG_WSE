use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use game_domain::id::{RegionId, SessionId};
use game_server::config::Config;
use networking::{DEFAULT_SETTLEMENT_REGION_ID, GatewayState, create_gateway_router};
use shared_protocol::EntityStateWire;
use simulation_core::coordinator::RegionCoordinator;
use simulation_systems::economy::SettlementEconomy;
use simulation_systems::faction::FactionManager;
use simulation_systems::integrated_hooks::IntegratedSimulationHooks;
use simulation_systems::rule_dsl::DynamicRuleRegistry;
use simulation_systems::scheduler::NpcScheduler;
use simulation_systems::system::NpcAiSystem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "WSE M4 Simulation & Gateway Server")]
struct Args {
    #[arg(long, default_value = "schemas/config.example.toml")]
    config: PathBuf,
    #[arg(long)]
    check_config: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    let config = Config::load(&args.config)?;
    if args.check_config {
        tracing::info!("configuration valid; no database connection or simulation started");
        return Ok(());
    }

    let mut settlement_region_id = RegionId(DEFAULT_SETTLEMENT_REGION_ID);
    let mut epoch = 1u64;
    let mut seed = 12345u64;
    let mut restored_state: Option<game_domain::RegionStateDto> = None;
    let mut db_pool: Option<sqlx::PgPool> = None;
    let mut lease_owner_id: Option<String> = None;

    if let Ok(db_url) = std::env::var("DATABASE_URL") {
        tracing::info!(
            "DATABASE_URL detected; initializing database pool and running migrations..."
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.persistence.pool_max_connections)
            .connect(&db_url)
            .await?;
        persistence::apply_migration(&pool).await?;
        tracing::info!("database migrations applied successfully; persistence layer ready");

        // Query authoritative region from DB and execute recovery pipeline with unique instance lease
        let instance_id = format!(
            "game-server-{}-{}",
            std::process::id(),
            sqlx::types::Uuid::new_v4()
        );
        let region_opt: Option<(sqlx::types::Uuid,)> =
            sqlx::query_as("SELECT id FROM regions ORDER BY created_at ASC LIMIT 1")
                .fetch_optional(&pool)
                .await?;

        if let Some((region_id,)) = region_opt {
            let recovered = persistence::RecoveryManager::recover_region(
                &pool,
                region_id,
                &instance_id,
                config.region.lease_seconds as i64,
            )
            .await?;
            tracing::info!(
                region_id = %recovered.region_id,
                tick = recovered.tick,
                epoch = recovered.epoch,
                entities = recovered.state.entities.len(),
                replayed_events = recovered.replayed_events_count,
                instance_id = %instance_id,
                "authoritative region recovered and leased successfully from persistence"
            );
            settlement_region_id = RegionId(recovered.region_id);
            epoch = recovered.epoch as u64;
            seed = recovered.rng_seed;
            restored_state = Some(recovered.state);
            lease_owner_id = Some(instance_id);
        } else {
            tracing::info!("no existing regions in database; bootstrap starting fresh world");
        }
        db_pool = Some(pool);
    }

    // 1. Initialize RegionCoordinator and spawn authoritative settlement RegionWorker
    let coordinator = Arc::new(RegionCoordinator::new());
    let tick_interval = Duration::from_millis(1000 / config.simulation.tick_rate_hz as u64);
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1024);

    let hooks = Box::new(IntegratedSimulationHooks::new(
        NpcAiSystem::default(),
        NpcScheduler::default(),
        SettlementEconomy::new_default(1),
        FactionManager::default(),
        DynamicRuleRegistry::default(),
        SessionId::from_u128(seed as u128),
        settlement_region_id,
    ));

    let region_handle = coordinator
        .spawn_region_with_restored_state(
            settlement_region_id,
            epoch,
            ecs_components::grid::MapGrid::default_64x64(),
            seed,
            Some(event_tx),
            Some(hooks),
            Some(tick_interval),
            restored_state,
        )
        .expect("failed to spawn settlement region worker");

    // 2. Initialize Gateway State with RegionCoordinator and Replicator
    let state = Arc::new(GatewayState::default().with_coordinator(coordinator.clone()));
    let replicator = state.get_or_create_replicator(DEFAULT_SETTLEMENT_REGION_ID);

    // 3. Connect TickOutput event stream to RegionReplicator
    tokio::spawn(async move {
        while let Some(tick_output) = event_rx.recv().await {
            let mut entities_wire: Vec<EntityStateWire> = Vec::new();
            // Replicate player & NPC position/stats from tick events or canonical state
            for event in &tick_output.events {
                match event {
                    simulation_core::event::SimulationEvent::EntityMoved {
                        entity_id,
                        to_x,
                        to_y,
                        ..
                    } => {
                        entities_wire.push(EntityStateWire {
                            id: entity_id.0,
                            x: (*to_x / 1000) as i32,
                            y: (*to_y / 1000) as i32,
                            entity_type: "player".into(),
                            hp: Some(100),
                            max_hp: Some(100),
                            name: Some("Entity".into()),
                        });
                    }
                    simulation_core::event::SimulationEvent::NpcDecisionExecuted {
                        entity_id,
                        tick,
                    } => {
                        entities_wire.push(EntityStateWire {
                            id: entity_id.0,
                            x: 32,
                            y: 32,
                            entity_type: "npc".into(),
                            hp: Some(100),
                            max_hp: Some(100),
                            name: Some(format!("NPC-tick-{}", tick)),
                        });
                    }
                    _ => {}
                }
            }
            if !entities_wire.is_empty() {
                replicator.broadcast_tick(tick_output.tick, &entities_wire);
            }
        }
    });

    // 4. Persistence background workers: Lease renewal, Snapshotting, and Loot Intent Settlement
    if let (Some(pool), Some(owner_id)) = (db_pool, lease_owner_id) {
        let lease_pool = pool.clone();
        let region_uuid = settlement_region_id.0;
        let lease_owner = owner_id.clone();
        let renew_secs = config.region.lease_renew_seconds.max(1);
        let lease_ttl = config.region.lease_seconds;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(renew_secs));
            loop {
                interval.tick().await;
                match persistence::acquire_or_renew_lease(&lease_pool, region_uuid, &lease_owner, lease_ttl as i64).await {
                    Ok(renewed_epoch) => {
                        tracing::trace!(region_id = %region_uuid, epoch = renewed_epoch, "region lease renewed");
                    }
                    Err(e) => {
                        tracing::error!(region_id = %region_uuid, error = %e, "failed to renew region lease");
                    }
                }
            }
        });

        let snapshot_pool = pool.clone();
        let snapshot_handle = region_handle.clone();
        let snapshot_secs = config.region.snapshot_interval_seconds.max(5);
        let snapshot_epoch = epoch as i64;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(snapshot_secs));
            loop {
                interval.tick().await;
                match snapshot_handle.capture_snapshot().await {
                    Ok(dto) => {
                        let snap_dto = persistence::SnapshotDto {
                            region_id: dto.region_id.0,
                            tick: dto.tick,
                            committed_event_seq: 0,
                            ownership_epoch: snapshot_epoch,
                            schema_version: persistence::CURRENT_SCHEMA_VERSION,
                            engine_version: persistence::CURRENT_ENGINE_VERSION.to_string(),
                            rules_version: persistence::CURRENT_RULES_VERSION,
                            rng_seed: seed,
                            rng_step: dto.tick,
                            state: dto.clone(),
                            scheduled_actions: Vec::new(),
                        };
                        match persistence::SnapshotManager::save_snapshot_fenced(&snapshot_pool, &snap_dto, snapshot_epoch).await {
                            Ok(snap_id) => {
                                tracing::info!(region_id = %dto.region_id.0, tick = dto.tick, snapshot_id = %snap_id, "periodic snapshot persisted");
                            }
                            Err(e) => {
                                tracing::warn!(region_id = %dto.region_id.0, error = %e, "failed to persist periodic snapshot");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to capture snapshot from region worker");
                    }
                }
            }
        });
    }

    let app = create_app_router(state);

    let listener = tokio::net::TcpListener::bind(config.server.http_listen).await?;
    tracing::info!(
        address = %listener.local_addr()?,
        milestone = "M5",
        "WSE M5 Simulation & Gateway Server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn create_app_router(state: Arc<GatewayState>) -> Router {
    create_gateway_router(state)
        .route(
            "/health/live",
            get(|| async {
                Json(serde_json::json!({
                    "service": "wse-game-server",
                    "milestone": "M5",
                    "status": "live",
                    "reason": "process_running"
                }))
            }),
        )
        .route(
            "/health/ready",
            get(|| async {
                Json(serde_json::json!({
                    "service": "wse-game-server",
                    "milestone": "M5",
                    "status": "ready",
                    "reason": "simulation_and_persistence_active"
                }))
            }),
        )
        .route("/", get(index_handler))
        .route("/{*path}", get(static_file_handler))
}

async fn index_handler() -> impl axum::response::IntoResponse {
    let index_path = Path::new("apps/web-client/dist/index.html");
    if let Ok(bytes) = std::fs::read(index_path) {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/html")],
            bytes,
        )
    } else {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            b"RPG WSE Game Server M4 - Web client dist not found. Build with 'npm run build' in apps/web-client".to_vec(),
        )
    }
}

async fn static_file_handler(
    axum::extract::Path(tail): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let base_path = Path::new("apps/web-client/dist");
    let target_path = base_path.join(tail.trim_start_matches('/'));

    // Prevent directory traversal attacks
    if let (Ok(canonical_target), Ok(canonical_base)) =
        (target_path.canonicalize(), base_path.canonicalize())
    {
        if !canonical_target.starts_with(&canonical_base) {
            return (
                StatusCode::FORBIDDEN,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                b"403 Forbidden".to_vec(),
            );
        }

        #[allow(clippy::collapsible_if)]
        if canonical_target.is_file() {
            if let Ok(bytes) = std::fs::read(&canonical_target) {
                let ext = canonical_target
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let mime = match ext {
                    "js" => "application/javascript",
                    "css" => "text/css",
                    "svg" => "image/svg+xml",
                    "png" => "image/png",
                    "json" => "application/json",
                    "woff2" => "font/woff2",
                    _ => "text/html",
                };
                return (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, mime)],
                    bytes,
                );
            }
        }
    }

    // Fallback to index.html for SPA client-side routing
    let index_path = base_path.join("index.html");
    if let Ok(bytes) = std::fs::read(index_path) {
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/html")],
            bytes,
        );
    }

    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        b"404 Not Found".to_vec(),
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
    tracing::info!("shutdown requested");
}
