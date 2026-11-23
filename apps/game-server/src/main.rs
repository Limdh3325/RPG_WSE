use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use game_server::config::Config;
use networking::{DEFAULT_SETTLEMENT_REGION_ID, GatewayState, create_gateway_router};
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
        } else {
            tracing::info!("no existing regions in database; bootstrap starting fresh world");
        }
    }

    // 1. Initialize Gateway State with Settlement Live Simulation
    let state =
        Arc::new(GatewayState::default().with_settlement_simulation(DEFAULT_SETTLEMENT_REGION_ID));

    // 2. Spawn 50ms tick loop for live simulation and AOI delta replication
    let state_for_tick = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        loop {
            interval.tick().await;
            if let Some(ref sim) = state_for_tick.simulation {
                sim.step_tick();
            }
        }
    });

    let app = create_app_router(state);

    let listener = tokio::net::TcpListener::bind(config.server.http_listen).await?;
    tracing::info!(
        address = %listener.local_addr()?,
        milestone = "M4",
        "WSE M4 Simulation & Gateway Server listening"
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
                    "milestone": "M4",
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
                    "milestone": "M4",
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
