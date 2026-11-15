//! Faction relationships, reputation matrices, and security dynamics.
//!
//! Enforces:
//! - Bounded reputation and standing values ([-100, 100])
//! - Symmetric or asymmetric faction hostility
//! - Standing-based price modifiers and guard aggression rules

use game_domain::EntityId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FactionId {
    Settlers,
    MerchantsGuild,
    Bandits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactionTier {
    Hostile,    // -100..=-50
    Distrusted, // -49..=-1
    Neutral,    // 0..=29
    Friendly,   // 30..=69
    Honored,    // 70..=100
}

impl FactionTier {
    pub fn from_standing(standing: i32) -> Self {
        match standing {
            s if s <= -50 => Self::Hostile,
            s if s < 0 => Self::Distrusted,
            s if s < 30 => Self::Neutral,
            s if s < 70 => Self::Friendly,
            _ => Self::Honored,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactionManager {
    relations: HashMap<(FactionId, FactionId), i32>,
    reputations: HashMap<(EntityId, FactionId), i32>,
}

impl Default for FactionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FactionManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            relations: HashMap::new(),
            reputations: HashMap::new(),
        };

        // Default inter-faction relations
        mgr.set_relation(FactionId::Settlers, FactionId::MerchantsGuild, 60);
        mgr.set_relation(FactionId::Settlers, FactionId::Bandits, -90);
        mgr.set_relation(FactionId::MerchantsGuild, FactionId::Bandits, -90);

        mgr
    }

    pub fn set_relation(&mut self, a: FactionId, b: FactionId, standing: i32) {
        let clamped = standing.clamp(-100, 100);
        self.relations.insert((a, b), clamped);
        self.relations.insert((b, a), clamped);
    }

    pub fn get_relation(&self, a: FactionId, b: FactionId) -> i32 {
        if a == b {
            return 100;
        }
        self.relations.get(&(a, b)).copied().unwrap_or(0)
    }

    pub fn get_reputation(&self, entity_id: EntityId, faction: FactionId) -> i32 {
        self.reputations
            .get(&(entity_id, faction))
            .copied()
            .unwrap_or(0)
    }

    pub fn adjust_reputation(
        &mut self,
        entity_id: EntityId,
        faction: FactionId,
        delta: i32,
    ) -> i32 {
        let current = self.get_reputation(entity_id, faction);
        let new_val = (current + delta).clamp(-100, 100);
        self.reputations.insert((entity_id, faction), new_val);
        new_val
    }

    /// Price discount factor in basis points (-2000 to +2000 => -20% to +20%)
    pub fn price_modifier_bp(&self, entity_id: EntityId, faction: FactionId) -> i32 {
        let rep = self.get_reputation(entity_id, faction);
        match FactionTier::from_standing(rep) {
            FactionTier::Honored => -1500, // 15% discount
            FactionTier::Friendly => -500, // 5% discount
            FactionTier::Neutral => 0,
            FactionTier::Distrusted => 1000, // 10% surcharge
            FactionTier::Hostile => 3000,    // 30% surcharge (or denied)
        }
    }

    pub fn is_hostile_to(&self, entity_id: EntityId, faction: FactionId) -> bool {
        self.get_reputation(entity_id, faction) <= -50
    }
}
