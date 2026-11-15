//! Strongly typed identifiers for the RPG World Simulation Engine.

use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};
use uuid::Uuid;

/// Strongly typed entity identifier (players, NPCs, monsters, interactive objects).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId(pub Uuid);

impl EntityId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::nil()
    }
}

impl Display for EntityId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for EntityId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<EntityId> for Uuid {
    fn from(id: EntityId) -> Self {
        id.0
    }
}

/// Strongly typed region identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegionId(pub Uuid);

impl RegionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for RegionId {
    fn default() -> Self {
        Self::nil()
    }
}

impl Display for RegionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for RegionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<RegionId> for Uuid {
    fn from(id: RegionId) -> Self {
        id.0
    }
}

/// Strongly typed faction identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FactionId(pub Uuid);

impl FactionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Display for FactionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for FactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for FactionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<FactionId> for Uuid {
    fn from(id: FactionId) -> Self {
        id.0
    }
}

/// Strongly typed command identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommandId(pub Uuid);

impl CommandId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for CommandId {
    fn default() -> Self {
        Self::nil()
    }
}

impl Display for CommandId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for CommandId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<CommandId> for Uuid {
    fn from(id: CommandId) -> Self {
        id.0
    }
}

impl From<u64> for CommandId {
    fn from(val: u64) -> Self {
        Self(Uuid::from_u128(val as u128))
    }
}

/// Strongly typed domain event identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Generates a deterministic event identifier for a region, tick, and sequence number.
    #[must_use]
    pub fn deterministic(region_id: RegionId, tick: u64, event_seq: u64) -> Self {
        let r = region_id.0.as_u128();
        let t = (tick as u128) << 32;
        let s = event_seq as u128;
        let mut mixed = r ^ (t.rotate_left(17)) ^ (s.rotate_left(43));
        mixed = mixed.wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835);
        mixed ^= mixed >> 64;
        Self(Uuid::from_u128(mixed))
    }

    /// Generates a deterministic child event identifier from a parent event and child index.
    #[must_use]
    pub fn deterministic_child(parent_id: EventId, child_index: usize) -> Self {
        let p = parent_id.0.as_u128();
        let c = child_index as u128;
        let mut mixed = p ^ (c.rotate_left(29));
        mixed = mixed.wrapping_mul(0x6A09_E667_F3BC_C908_BB67_AE85_84CA_A73B);
        mixed ^= mixed >> 64;
        Self(Uuid::from_u128(mixed))
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::nil()
    }
}

impl Display for EventId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for EventId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<EventId> for Uuid {
    fn from(id: EventId) -> Self {
        id.0
    }
}

/// Strongly typed client/session identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(Uuid::from_u128(v))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::nil()
    }
}

impl Display for SessionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for SessionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<SessionId> for Uuid {
    fn from(id: SessionId) -> Self {
        id.0
    }
}
