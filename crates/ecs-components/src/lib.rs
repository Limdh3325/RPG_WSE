//! Region ECS components, grid layout, StableId indexing, and World management.
//!
//! Part of the RPG World Simulation Engine (WSE).

pub mod components;
pub mod grid;
pub mod index;
pub mod region_world;

pub use components::{
    AIState, Collider, CombatStats, FactionMembership, Health, InventoryProjection, MemoryRef,
    MonsterArchetype, Navigation, Needs, NpcMemoryEntry, NpcRelationshipEntry, Personality,
    Position, StableId, Velocity,
};
pub use grid::{DEFAULT_GRID_HEIGHT_TILES, DEFAULT_GRID_WIDTH_TILES, MapGrid};
pub use index::StableEntityIndex;
pub use region_world::{RegionMetaResource, RegionWorld};
