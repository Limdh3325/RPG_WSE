//! Canonical state DTOs and lifecycle enums.
//!
//! These structures detach domain state from Bevy ECS internal handles,
//! allowing clean serialization, persistence, network replication, and
//! cross-restart restoration.

use crate::id::{EntityId, RegionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle state of a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegionStatus {
    Loading,
    Active,
    Background,
    Migrating,
    Paused,
    Stopped,
}

impl Default for RegionStatus {
    fn default() -> Self {
        Self::Loading
    }
}

/// Operational health indicator of a region runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegionHealth {
    Healthy,
    Degraded,
    Failed,
}

impl Default for RegionHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

/// Combat status of an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CombatStatus {
    Alive,
    Downed,
    Dead,
}

impl Default for CombatStatus {
    fn default() -> Self {
        Self::Alive
    }
}

/// Navigation mode/state of an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NavigationState {
    Idle,
    PathRequested,
    Following,
    Blocked,
    Arrived,
    Cancelled,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Canonical DTO for entity position in millitiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PositionDto {
    pub x_millitile: i64,
    pub y_millitile: i64,
}

/// Canonical DTO for velocity in millitiles per tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VelocityDto {
    pub dx: i32,
    pub dy: i32,
}

/// Canonical DTO for health and alive/dead state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HealthDto {
    pub current: i32,
    pub max: i32,
    pub status: CombatStatus,
}

/// Canonical DTO for survival needs (0..=1000 clamped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NeedsDto {
    pub hunger: u16,
    pub fatigue: u16,
    pub safety: u16,
}

/// Canonical DTO for personality traits (-1000..=1000 clamped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonalityDto {
    pub courage: i16,
    pub greed: i16,
    pub loyalty: i16,
}

/// Canonical DTO for an inventory slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InventorySlotDto {
    pub slot_index: u16,
    pub item_id: String,
    pub quantity: u32,
}

/// Canonical DTO for committed inventory projection.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InventoryProjectionDto {
    pub committed_version: u64,
    pub slots: Vec<InventorySlotDto>,
}

/// Canonical DTO for faction membership and standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FactionMembershipDto {
    pub faction_id: Uuid,
    pub standing: i32,
}

/// Canonical DTO for physical collider bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColliderDto {
    pub width_millitile: u32,
    pub height_millitile: u32,
    pub is_trigger: bool,
}

/// Canonical complete snapshot representation of an entity, detached from Bevy handles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityStateDto {
    pub id: EntityId,
    pub position: PositionDto,
    pub velocity: Option<VelocityDto>,
    pub health: Option<HealthDto>,
    pub needs: Option<NeedsDto>,
    pub personality: Option<PersonalityDto>,
    pub inventory: Option<InventoryProjectionDto>,
    pub faction: Option<FactionMembershipDto>,
    pub collider: Option<ColliderDto>,
}

/// Canonical complete snapshot representation of a region and its entities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionStateDto {
    pub region_id: RegionId,
    pub epoch: u64,
    pub tick: u64,
    pub status: RegionStatus,
    pub entities: Vec<EntityStateDto>,
}
