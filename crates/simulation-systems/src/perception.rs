//! NPC Perception system.
//!
//! Scans the surrounding spatial region to extract atomic world facts
//! for Utility AI and GOAP decision making.

use ecs_components::{
    CombatStats, FactionMembership, InventoryProjection, MonsterArchetype, Position,
};
use game_domain::{EntityId, FactionId, RegionId, SessionId};

/// Configuration for NPC sensory perception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerceptionConfig {
    /// Vision radius in millitiles (1 tile = 1,000 millitiles, default 10 tiles).
    pub vision_radius_millitiles: i64,
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            vision_radius_millitiles: 10_000,
        }
    }
}

/// Atomic facts observed by an NPC from perception and internal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NpcWorldFacts {
    /// Whether an active hostile threat is observed within vision range.
    pub threat_present: bool,
    /// Estimated threat level (0..=1000).
    pub threat_level: u16,
    /// Entity ID of the most pressing threat, if any.
    pub threat_entity_id: Option<EntityId>,
    /// Position of the most pressing threat, if known.
    pub threat_pos: Option<Position>,
    /// Whether the NPC is currently safe (i.e. no threat in range).
    pub is_safe: bool,
    /// Whether the NPC is actively taking damage or locked in combat.
    pub in_combat: bool,
    /// Whether the NPC possesses edible food in their inventory.
    pub has_food: bool,
    /// Whether the NPC possesses gold in their inventory.
    pub has_gold: bool,
    /// Whether an open shop or merchant is accessible.
    pub shop_open: bool,
    /// Whether the NPC is currently at or near the marketplace/shop.
    pub at_market: bool,
    /// Whether the NPC is currently at or near their home location.
    pub near_home: bool,
    /// Internal satisfaction flag: hunger reduced.
    pub hunger_reduced: bool,
    /// Internal satisfaction flag: fatigue reduced.
    pub fatigue_reduced: bool,
    /// Internal satisfaction flag: daily work quota satisfied.
    pub work_done: bool,
}

/// Perception context for evaluating surrounding entities.
pub struct PerceptionContext<'a> {
    pub self_entity_id: EntityId,
    pub self_pos: Position,
    pub self_faction: Option<FactionId>,
    pub inventory: Option<&'a InventoryProjection>,
    pub market_pos: Option<Position>,
    pub home_pos: Option<Position>,
    pub region_id: RegionId,
    pub session_id: SessionId,
}

/// Surrounding entity tuple for perception queries.
pub type SurroundingEntityData = (
    EntityId,
    Position,
    Option<FactionMembership>,
    Option<CombatStats>,
    Option<MonsterArchetype>,
);

/// Extracts world facts for a given NPC by evaluating surrounding entities.
#[must_use]
pub fn extract_world_facts(
    ctx: &PerceptionContext<'_>,
    surrounding_entities: &[SurroundingEntityData],
    config: &PerceptionConfig,
) -> NpcWorldFacts {
    let mut facts = NpcWorldFacts::default();

    // 1. Inventory evaluation (Food & Gold)
    if let Some(inv) = ctx.inventory {
        for slot in &inv.slots {
            let item_lower = slot.item_id.to_ascii_lowercase();
            if (item_lower.contains("food")
                || item_lower.contains("bread")
                || item_lower.contains("apple")
                || item_lower.contains("ration"))
                && slot.quantity > 0
            {
                facts.has_food = true;
            }
            if (item_lower.contains("gold") || item_lower.contains("coin")) && slot.quantity > 0 {
                facts.has_gold = true;
            }
        }
    }

    // 2. Spatial evaluation: Market & Home proximity (threshold: 3 tiles = 3,000 millitiles)
    let proximity_threshold_sq = 3_000 * 3_000;
    if let Some(market) = ctx.market_pos {
        let dx = ctx.self_pos.x_millitile - market.x_millitile;
        let dy = ctx.self_pos.y_millitile - market.y_millitile;
        if dx * dx + dy * dy <= proximity_threshold_sq {
            facts.at_market = true;
        }
    }
    if let Some(home) = ctx.home_pos {
        let dx = ctx.self_pos.x_millitile - home.x_millitile;
        let dy = ctx.self_pos.y_millitile - home.y_millitile;
        if dx * dx + dy * dy <= proximity_threshold_sq {
            facts.near_home = true;
        }
    }

    // 3. Surrounding entity scan for threats & open merchants
    let vision_radius_sq = config.vision_radius_millitiles * config.vision_radius_millitiles;
    let mut max_threat_severity: u16 = 0;
    let mut closest_threat_id = None;
    let mut closest_threat_pos = None;

    for (other_id, other_pos, other_faction, other_combat, other_monster) in surrounding_entities {
        if *other_id == ctx.self_entity_id {
            continue;
        }

        let dx = ctx.self_pos.x_millitile - other_pos.x_millitile;
        let dy = ctx.self_pos.y_millitile - other_pos.y_millitile;
        let dist_sq = dx * dx + dy * dy;

        if dist_sq > vision_radius_sq {
            continue;
        }

        // Threat detection: hostile monster archetype or opposing faction with negative standing
        let is_hostile = match (ctx.self_faction, other_faction) {
            (Some(sf), Some(of)) => {
                sf.0 != of.faction_id && (of.standing < 0 || other_monster.is_some())
            }
            (_, Some(of)) => of.standing < 0 || other_monster.is_some(),
            (_, None) => other_monster.is_some(),
        };

        if is_hostile {
            let base_threat = if let Some(combat) = other_combat {
                (combat.base_damage.max(1) as u16) * 40
            } else {
                300
            };
            let severity = base_threat.min(1000);
            if severity > max_threat_severity {
                max_threat_severity = severity;
                closest_threat_id = Some(*other_id);
                closest_threat_pos = Some(*other_pos);
            }
        }
    }

    if max_threat_severity > 0 {
        facts.threat_present = true;
        facts.threat_level = max_threat_severity;
        facts.threat_entity_id = closest_threat_id;
        facts.threat_pos = closest_threat_pos;
        facts.is_safe = false;
    } else {
        facts.threat_present = false;
        facts.threat_level = 0;
        facts.threat_entity_id = None;
        facts.threat_pos = None;
        facts.is_safe = true;
    }

    // Shop availability: accessible if a market exists and is not under direct threat
    facts.shop_open = ctx.market_pos.is_some() && !facts.threat_present;

    facts
}
