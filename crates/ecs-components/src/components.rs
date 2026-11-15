//! Core ECS components for the RPG World Simulation Engine.

use bevy_ecs::component::Component;
use game_domain::{
    ColliderDto, CombatStatus, DomainError, EntityId, FactionMembershipDto, HealthDto,
    InventoryProjectionDto, InventorySlotDto, NavigationState, NeedsDto, PersonalityDto,
    PositionDto, VelocityDto, millitiles_to_tile,
};
use uuid::Uuid;

/// Stable, persistent entity identity (UUID).
/// Mapped bi-directionally to Bevy ECS Entity handles.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableId(pub EntityId);

impl StableId {
    #[must_use]
    pub const fn new(id: EntityId) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.0
    }
}

/// World position in integer millitiles (1 tile = 1,000 millitiles).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Position {
    pub x_millitile: i64,
    pub y_millitile: i64,
}

impl Position {
    #[must_use]
    pub const fn new(x_millitile: i64, y_millitile: i64) -> Self {
        Self {
            x_millitile,
            y_millitile,
        }
    }

    #[must_use]
    pub const fn tile_x(&self) -> i64 {
        millitiles_to_tile(self.x_millitile)
    }

    #[must_use]
    pub const fn tile_y(&self) -> i64 {
        millitiles_to_tile(self.y_millitile)
    }

    #[must_use]
    pub fn distance_squared(&self, other: &Self) -> u128 {
        let dx = (self.x_millitile as i128 - other.x_millitile as i128).unsigned_abs();
        let dy = (self.y_millitile as i128 - other.y_millitile as i128).unsigned_abs();
        (dx * dx).saturating_add(dy * dy)
    }

    #[must_use]
    pub fn distance_manhattan(&self, other: &Self) -> u128 {
        let dx = (self.x_millitile as i128 - other.x_millitile as i128).unsigned_abs();
        let dy = (self.y_millitile as i128 - other.y_millitile as i128).unsigned_abs();
        dx.saturating_add(dy)
    }

    #[must_use]
    pub const fn from_dto(dto: PositionDto) -> Self {
        Self {
            x_millitile: dto.x_millitile,
            y_millitile: dto.y_millitile,
        }
    }

    #[must_use]
    pub const fn to_dto(&self) -> PositionDto {
        PositionDto {
            x_millitile: self.x_millitile,
            y_millitile: self.y_millitile,
        }
    }
}

/// Velocity in millitiles per tick.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Velocity {
    pub dx: i32,
    pub dy: i32,
}

impl Velocity {
    #[must_use]
    pub const fn new(dx: i32, dy: i32) -> Self {
        Self { dx, dy }
    }

    #[must_use]
    pub fn speed_squared(&self) -> u64 {
        let x = self.dx as i64;
        let y = self.dy as i64;
        (x * x + y * y) as u64
    }

    #[must_use]
    pub const fn from_dto(dto: VelocityDto) -> Self {
        Self {
            dx: dto.dx,
            dy: dto.dy,
        }
    }

    #[must_use]
    pub const fn to_dto(&self) -> VelocityDto {
        VelocityDto {
            dx: self.dx,
            dy: self.dy,
        }
    }
}

/// Health and vitality state.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Health {
    pub current: i32,
    pub max: i32,
    pub status: CombatStatus,
}

impl Health {
    pub fn new(current: i32, max: i32) -> Result<Self, DomainError> {
        if max <= 0 || current > max || current < 0 {
            return Err(DomainError::InvalidHealth { current, max });
        }
        let status = if current == 0 {
            CombatStatus::Dead
        } else {
            CombatStatus::Alive
        };
        Ok(Self {
            current,
            max,
            status,
        })
    }

    #[must_use]
    pub const fn is_alive(&self) -> bool {
        matches!(self.status, CombatStatus::Alive) && self.current > 0
    }

    pub fn apply_damage(&mut self, amount: i32) -> CombatStatus {
        if amount <= 0 {
            return self.status;
        }
        self.current = (self.current - amount).max(0);
        if self.current == 0 {
            self.status = CombatStatus::Dead;
        }
        self.status
    }

    pub fn heal(&mut self, amount: i32) {
        if amount <= 0 || !self.is_alive() {
            return;
        }
        self.current = (self.current + amount).min(self.max);
    }

    pub fn from_dto(dto: HealthDto) -> Result<Self, DomainError> {
        if dto.max <= 0 || dto.current < 0 || dto.current > dto.max {
            return Err(DomainError::InvalidHealth {
                current: dto.current,
                max: dto.max,
            });
        }
        if dto.current == 0 && dto.status == CombatStatus::Alive {
            return Err(DomainError::InvalidHealth {
                current: dto.current,
                max: dto.max,
            });
        }
        if dto.current > 0 && dto.status == CombatStatus::Dead {
            return Err(DomainError::InvalidHealth {
                current: dto.current,
                max: dto.max,
            });
        }
        Ok(Self {
            current: dto.current,
            max: dto.max,
            status: dto.status,
        })
    }

    #[must_use]
    pub const fn to_dto(&self) -> HealthDto {
        HealthDto {
            current: self.current,
            max: self.max,
            status: self.status,
        }
    }
}

/// Survival needs (0..=1000 clamped).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Needs {
    pub hunger: u16,
    pub fatigue: u16,
    pub safety: u16,
}

impl Needs {
    pub const MAX_VALUE: u16 = 1000;

    #[must_use]
    pub fn new(hunger: u16, fatigue: u16, safety: u16) -> Self {
        Self {
            hunger: hunger.min(Self::MAX_VALUE),
            fatigue: fatigue.min(Self::MAX_VALUE),
            safety: safety.min(Self::MAX_VALUE),
        }
    }

    pub fn clamp(&mut self) {
        self.hunger = self.hunger.min(Self::MAX_VALUE);
        self.fatigue = self.fatigue.min(Self::MAX_VALUE);
        self.safety = self.safety.min(Self::MAX_VALUE);
    }

    #[must_use]
    pub const fn from_dto(dto: NeedsDto) -> Self {
        Self {
            hunger: if dto.hunger > Self::MAX_VALUE {
                Self::MAX_VALUE
            } else {
                dto.hunger
            },
            fatigue: if dto.fatigue > Self::MAX_VALUE {
                Self::MAX_VALUE
            } else {
                dto.fatigue
            },
            safety: if dto.safety > Self::MAX_VALUE {
                Self::MAX_VALUE
            } else {
                dto.safety
            },
        }
    }

    #[must_use]
    pub const fn to_dto(&self) -> NeedsDto {
        NeedsDto {
            hunger: self.hunger,
            fatigue: self.fatigue,
            safety: self.safety,
        }
    }
}

impl Default for Needs {
    fn default() -> Self {
        Self {
            hunger: 0,
            fatigue: 0,
            safety: Self::MAX_VALUE,
        }
    }
}

/// Personality traits (-1000..=1000 clamped).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Personality {
    pub courage: i16,
    pub greed: i16,
    pub loyalty: i16,
}

impl Personality {
    pub const MIN_VALUE: i16 = -1000;
    pub const MAX_VALUE: i16 = 1000;

    #[must_use]
    pub fn new(courage: i16, greed: i16, loyalty: i16) -> Self {
        Self {
            courage: courage.clamp(Self::MIN_VALUE, Self::MAX_VALUE),
            greed: greed.clamp(Self::MIN_VALUE, Self::MAX_VALUE),
            loyalty: loyalty.clamp(Self::MIN_VALUE, Self::MAX_VALUE),
        }
    }

    #[must_use]
    pub fn from_dto(dto: PersonalityDto) -> Self {
        Self::new(dto.courage, dto.greed, dto.loyalty)
    }

    #[must_use]
    pub const fn to_dto(&self) -> PersonalityDto {
        PersonalityDto {
            courage: self.courage,
            greed: self.greed,
            loyalty: self.loyalty,
        }
    }
}

/// Navigation target, waypoints, and status.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct Navigation {
    pub target_x_millitile: Option<i64>,
    pub target_y_millitile: Option<i64>,
    pub waypoints: Vec<(i64, i64)>,
    pub current_waypoint_idx: usize,
    pub path_version: u32,
    pub state: NavigationState,
    pub speed_millitiles_per_tick: i64,
    pub failure_reason: Option<String>,
}

impl Navigation {
    #[must_use]
    pub fn new(speed_millitiles_per_tick: i64) -> Self {
        Self {
            speed_millitiles_per_tick,
            ..Default::default()
        }
    }
}

/// Committed inventory projection for an entity.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct InventoryProjection {
    pub committed_version: u64,
    pub slots: Vec<InventorySlotDto>,
}

impl InventoryProjection {
    #[must_use]
    pub const fn new(committed_version: u64, slots: Vec<InventorySlotDto>) -> Self {
        Self {
            committed_version,
            slots,
        }
    }

    #[must_use]
    pub fn from_dto(dto: InventoryProjectionDto) -> Self {
        Self {
            committed_version: dto.committed_version,
            slots: dto.slots,
        }
    }

    #[must_use]
    pub fn to_dto(&self) -> InventoryProjectionDto {
        InventoryProjectionDto {
            committed_version: self.committed_version,
            slots: self.slots.clone(),
        }
    }
}

/// Faction membership and standing.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FactionMembership {
    pub faction_id: Uuid,
    pub standing: i32,
}

impl FactionMembership {
    #[must_use]
    pub const fn new(faction_id: Uuid, standing: i32) -> Self {
        Self {
            faction_id,
            standing,
        }
    }

    #[must_use]
    pub const fn from_dto(dto: FactionMembershipDto) -> Self {
        Self {
            faction_id: dto.faction_id,
            standing: dto.standing,
        }
    }

    #[must_use]
    pub const fn to_dto(&self) -> FactionMembershipDto {
        FactionMembershipDto {
            faction_id: self.faction_id,
            standing: self.standing,
        }
    }
}

/// Physical collider and obstruction bounds.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Collider {
    pub width_millitile: u32,
    pub height_millitile: u32,
    pub is_trigger: bool,
}

impl Collider {
    #[must_use]
    pub const fn new(width_millitile: u32, height_millitile: u32, is_trigger: bool) -> Self {
        Self {
            width_millitile,
            height_millitile,
            is_trigger,
        }
    }

    #[must_use]
    pub const fn from_dto(dto: ColliderDto) -> Self {
        Self {
            width_millitile: dto.width_millitile,
            height_millitile: dto.height_millitile,
            is_trigger: dto.is_trigger,
        }
    }

    #[must_use]
    pub const fn to_dto(&self) -> ColliderDto {
        ColliderDto {
            width_millitile: self.width_millitile,
            height_millitile: self.height_millitile,
            is_trigger: self.is_trigger,
        }
    }
}

/// High-level AI state for NPCs.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct AIState {
    pub goal: String,
    pub action: String,
    pub next_decision_tick: u64,
}

/// Combat attributes and cooldown tracking.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CombatStats {
    pub attack_range_millitile: i64,
    pub base_damage: i32,
    pub armor: i32,
    pub attack_cooldown_ticks: u64,
    pub last_attack_tick: u64,
}

impl CombatStats {
    #[must_use]
    pub const fn new(
        attack_range_millitile: i64,
        base_damage: i32,
        armor: i32,
        attack_cooldown_ticks: u64,
    ) -> Self {
        Self {
            attack_range_millitile,
            base_damage,
            armor,
            attack_cooldown_ticks,
            last_attack_tick: 0,
        }
    }

    #[must_use]
    pub const fn can_attack(&self, current_tick: u64) -> bool {
        self.last_attack_tick == 0
            || current_tick >= self.last_attack_tick + self.attack_cooldown_ticks
    }
}

/// Monster species archetype for PvE dungeon encounters.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MonsterArchetype {
    GoblinScout,
    OrcWarrior,
}

impl MonsterArchetype {
    #[must_use]
    pub const fn max_health(&self) -> i32 {
        match self {
            Self::GoblinScout => 30,
            Self::OrcWarrior => 80,
        }
    }

    #[must_use]
    pub const fn combat_stats(&self) -> CombatStats {
        match self {
            Self::GoblinScout => CombatStats::new(1200, 8, 1, 3),
            Self::OrcWarrior => CombatStats::new(1500, 18, 5, 6),
        }
    }

    #[must_use]
    pub const fn movement_speed_millitiles_per_tick(&self) -> i64 {
        match self {
            Self::GoblinScout => 300,
            Self::OrcWarrior => 150,
        }
    }

    #[must_use]
    pub const fn default_loot(&self) -> (u64, u32) {
        match self {
            Self::GoblinScout => (1001, 1),
            Self::OrcWarrior => (1002, 1),
        }
    }
}

/// Reference to NPC memories and social relationships.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct MemoryRef {
    pub memories: Vec<NpcMemoryEntry>,
    pub relationships: Vec<NpcRelationshipEntry>,
}

/// An individual memory item held by an NPC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcMemoryEntry {
    pub source_event_id: Option<Uuid>,
    pub kind: String,
    pub target_id: Option<EntityId>,
    pub severity: i32,
    pub happened_tick: u64,
    pub ttl_ticks: u64,
}

/// Relationship evaluation toward another entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcRelationshipEntry {
    pub target_id: EntityId,
    pub trust: i32,
    pub affinity: i32,
    pub fear: i32,
}

impl MemoryRef {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            memories: Vec::new(),
            relationships: Vec::new(),
        }
    }

    /// Records a memory entry with deduplication on source_event_id.
    /// Returns true if recorded, false if ignored as duplicate.
    pub fn record_memory(&mut self, entry: NpcMemoryEntry) -> bool {
        let is_duplicate = entry.source_event_id.is_some_and(|src_id| {
            self.memories
                .iter()
                .any(|m| m.source_event_id == Some(src_id))
        });
        if is_duplicate {
            return false;
        }
        self.memories.push(entry);
        true
    }

    /// Prunes expired memories whose TTL has elapsed.
    pub fn prune_expired(&mut self, current_tick: u64) {
        self.memories
            .retain(|m| current_tick < m.happened_tick.saturating_add(m.ttl_ticks));
    }
}
