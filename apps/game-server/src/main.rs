use clap::Parser;
use game_server::config::Config;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "WSE M0 bootstrap server (gameplay not implemented)")]
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

    let listener = tokio::net::TcpListener::bind(config.server.http_listen).await?;
    tracing::info!(address = %listener.local_addr()?, milestone = "M0", "bootstrap listening; readiness unavailable");
    axum_serve(listener).await?;
    Ok(())
}

async fn axum_serve(listener: tokio::net::TcpListener) -> std::io::Result<()> {
    networking::serve(listener, shutdown_signal()).await
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
