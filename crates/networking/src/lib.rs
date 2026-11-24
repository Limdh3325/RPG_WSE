//! Networking and Area of Interest (AOI) replication layer for RPG WSE.

use axum::{Json, Router, http::StatusCode, routing::get};
use serde::Serialize;
use std::future::Future;
use std::sync::Arc;

pub mod aoi;
pub mod gateway;
pub mod replicator;
pub mod session;
pub mod simulation;
pub mod ticket;

pub use aoi::{AoiFilter, CELL_SIZE, CellCoord, INTEREST_RADIUS_CELLS};
pub use gateway::{AUTH_TIMEOUT, GatewayState, InboundCommand, create_gateway_router};
pub use replicator::RegionReplicator;
pub use session::{CommandRateLimiter, Session, SessionError, SharedSession};
pub use simulation::{DEFAULT_SETTLEMENT_REGION_ID, SettlementSimulation};
pub use ticket::{DEFAULT_TICKET_TTL, TicketData, TicketStore};

#[derive(Serialize)]
struct Health {
    service: &'static str,
    milestone: &'static str,
    status: &'static str,
    reason: &'static str,
}

fn health(status: &'static str, reason: &'static str) -> Json<Health> {
    Json(Health {
        service: "wse-game-server",
        milestone: "M0",
        status,
        reason,
    })
}

pub fn router() -> Router {
    create_gateway_router(Arc::new(GatewayState::default()))
        .route(
            "/health/live",
            get(|| async { health("live", "process_running") }),
        )
        .route(
            "/health/ready",
            get(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    health("not_ready", "simulation_and_persistence_not_initialized"),
                )
            }),
        )
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown)
        .await
}
