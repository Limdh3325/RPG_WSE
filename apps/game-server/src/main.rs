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
