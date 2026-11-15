//! Domain IDs, coordinates, canonical state DTOs, and domain errors.
//!
//! Part of the RPG World Simulation Engine (WSE).

pub mod coordinates;
pub mod errors;
pub mod id;
pub mod state;

pub use coordinates::{
    MILLITILES_PER_TILE, millitiles_to_tile, tile_to_center_millitile, tile_to_origin_millitile,
};
pub use errors::DomainError;
pub use id::{CommandId, EntityId, EventId, FactionId, RegionId, SessionId};
pub use state::{
    ColliderDto, CombatStatus, EntityStateDto, FactionMembershipDto, HealthDto,
    InventoryProjectionDto, InventorySlotDto, NavigationState, NeedsDto, PersonalityDto,
    PositionDto, RegionHealth, RegionStateDto, RegionStatus, VelocityDto,
};
