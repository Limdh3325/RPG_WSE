//! NPC Utility AI: Goals and non-linear score curves.
//!
//! Evaluates candidate goals based on WorldFacts, Needs, Personality, and hysteresis,
//! producing a deterministic top-ranked goal.

use crate::perception::NpcWorldFacts;
use ecs_components::{Needs, Personality};
use game_domain::EntityId;
use serde::{Deserialize, Serialize};

/// High-level goals selectable by Utility AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NpcGoal {
    Flee = 0,
    Eat = 1,
    Rest = 2,
    Work = 3,
    BuyFood = 4,
    Patrol = 5,
    Wander = 6,
    Idle = 7,
}

impl NpcGoal {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Flee => "Flee",
            Self::Eat => "Eat",
            Self::Rest => "Rest",
            Self::Work => "Work",
            Self::BuyFood => "BuyFood",
            Self::Patrol => "Patrol",
            Self::Wander => "Wander",
            Self::Idle => "Idle",
        }
    }
}

/// Utility AI scoring evaluator.
#[derive(Debug)]
pub struct UtilityEvaluator {
    /// Hysteresis threshold to prevent goal flapping / ping-ponging.
    pub switch_margin: u32,
}

impl Default for UtilityEvaluator {
    fn default() -> Self {
        Self { switch_margin: 100 }
    }
}

impl UtilityEvaluator {
    #[must_use]
    pub const fn new(switch_margin: u32) -> Self {
        Self { switch_margin }
    }

    /// Evaluates raw utility score for a goal in range 0..=1000.
    #[must_use]
    pub fn evaluate_score(
        &self,
        goal: NpcGoal,
        facts: &NpcWorldFacts,
        needs: &Needs,
        personality: &Personality,
    ) -> u32 {
        let raw_score = match goal {
            NpcGoal::Flee => {
                if facts.threat_present {
                    // Base 800 + threat_level / 5 - courage / 10
                    // Ensures that when threat_present is true (e.g. threat=700),
                    // Flee utility (~940) strictly dominates Eat (~830-850) (TEST-AI-001).
                    let threat_bonus = (i32::from(facts.threat_level) / 5).min(200);
                    let courage_discount = i32::from(personality.courage) / 10;
                    800 + threat_bonus - courage_discount
                } else {
                    0
                }
            }
            NpcGoal::Eat => {
                if facts.has_food {
                    let base = (f32::from(needs.hunger) * 0.95) as i32;
                    base + 50
                } else if facts.has_gold && facts.shop_open {
                    // Needs to buy food first, strong drive if hungry
                    (f32::from(needs.hunger) * 0.90) as i32 + 20
                } else {
                    (f32::from(needs.hunger) * 0.50) as i32
                }
            }
            NpcGoal::BuyFood => {
                if !facts.has_food && facts.has_gold && facts.shop_open && needs.hunger > 300 {
                    (f32::from(needs.hunger) * 0.85) as i32 + 30
                } else {
                    0
                }
            }
            NpcGoal::Rest => {
                if facts.threat_present {
                    // Suppressed under threat
                    (f32::from(needs.fatigue) * 0.20) as i32
                } else {
                    (f32::from(needs.fatigue) * 0.90) as i32
                }
            }
            NpcGoal::Work => {
                if facts.threat_present {
                    0
                } else {
                    let greed_bonus = i32::from(personality.greed) / 5;
                    let loyalty_bonus = i32::from(personality.loyalty) / 10;
                    250 + greed_bonus + loyalty_bonus
                }
            }
            NpcGoal::Patrol => {
                if personality.loyalty > 200 && !facts.threat_present {
                    200 + i32::from(personality.loyalty) / 10
                } else {
                    50
                }
            }
            NpcGoal::Wander => 100,
            NpcGoal::Idle => 50,
        };

        raw_score.clamp(0, 1000) as u32
    }

    /// Selects the best goal, applying hysteresis and deterministic tie-breaking.
    #[must_use]
    pub fn select_goal(
        &self,
        current_goal: Option<NpcGoal>,
        facts: &NpcWorldFacts,
        needs: &Needs,
        personality: &Personality,
        _stable_id: EntityId,
    ) -> (NpcGoal, u32) {
        let candidate_goals = [
            NpcGoal::Flee,
            NpcGoal::Eat,
            NpcGoal::BuyFood,
            NpcGoal::Rest,
            NpcGoal::Work,
            NpcGoal::Patrol,
            NpcGoal::Wander,
            NpcGoal::Idle,
        ];

        let mut scored_goals: Vec<(NpcGoal, u32)> = candidate_goals
            .iter()
            .map(|&g| (g, self.evaluate_score(g, facts, needs, personality)))
            .collect();

        // Deterministic sort: descending score, then ascending goal priority (lower enum discriminant wins)
        scored_goals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let (top_goal, top_score) = scored_goals[0];

        // Hysteresis logic:
        // Immediate escape: If threat is present and top_goal is Flee, switch immediately!
        if top_goal == NpcGoal::Flee && facts.threat_present {
            return (top_goal, top_score);
        }

        if let Some(curr) = current_goal {
            let current_score = self.evaluate_score(curr, facts, needs, personality);
            // Switch only if top_score > current_score + switch_margin
            if top_score <= current_score.saturating_add(self.switch_margin) {
                return (curr, current_score);
            }
        }

        (top_goal, top_score)
    }
}
