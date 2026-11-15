//! NPC Needs simulation and dynamics.
//!
//! Tracks and updates survival metrics (hunger, fatigue, safety, wealth_pressure)
//! clamped within the range 0..=1000.

use crate::perception::NpcWorldFacts;
use ecs_components::Needs;

/// Updates biological and psychological needs over simulation ticks.
pub fn update_needs(needs: &mut Needs, facts: &NpcWorldFacts, is_resting: bool, is_working: bool) {
    // 1. Hunger dynamics: steadily increases unless fed
    if facts.hunger_reduced {
        needs.hunger = needs.hunger.saturating_sub(400);
    } else {
        needs.hunger = (needs.hunger + 1).min(1000);
    }

    // 2. Fatigue dynamics: recovers when resting, accumulates when working or moving
    if is_resting || facts.fatigue_reduced {
        needs.fatigue = needs.fatigue.saturating_sub(15);
    } else if is_working {
        needs.fatigue = (needs.fatigue + 2).min(1000);
    } else {
        needs.fatigue = (needs.fatigue + 1).min(1000);
    }

    // 3. Safety dynamics: suppressed by threats, recovers in peaceful conditions
    if facts.threat_present {
        let threat_penalty = facts.threat_level.min(1000);
        needs.safety = 1000_u16.saturating_sub(threat_penalty);
    } else {
        needs.safety = (needs.safety + 10).min(1000);
    }
}
