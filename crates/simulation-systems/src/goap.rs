//! Goal-Oriented Action Planning (GOAP) engine.
//!
//! Includes A* state-space planning with deterministic tie-breaking,
//! hard bounds (`max_expansions=128`, `max_plan_length=12`),
//! and action precondition/effect graph chaining.

use crate::perception::NpcWorldFacts;
use crate::utility::NpcGoal;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

/// Compact representation of world facts for GOAP state-space search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GoapState {
    pub threat_present: bool,
    pub is_safe: bool,
    pub in_combat: bool,
    pub has_food: bool,
    pub hunger_reduced: bool,
    pub fatigue_reduced: bool,
    pub near_home: bool,
    pub has_gold: bool,
    pub shop_open: bool,
    pub at_market: bool,
    pub work_done: bool,
}

impl From<&NpcWorldFacts> for GoapState {
    fn from(facts: &NpcWorldFacts) -> Self {
        Self {
            threat_present: facts.threat_present,
            is_safe: facts.is_safe,
            in_combat: facts.in_combat,
            has_food: facts.has_food,
            hunger_reduced: facts.hunger_reduced,
            fatigue_reduced: facts.fatigue_reduced,
            near_home: facts.near_home,
            has_gold: facts.has_gold,
            shop_open: facts.shop_open,
            at_market: facts.at_market,
            work_done: facts.work_done,
        }
    }
}

/// Target goal condition for the GOAP planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoapGoal {
    Survive,
    Eat,
    Rest,
    Work,
    Patrol,
    Wander,
    Idle,
}

impl From<NpcGoal> for GoapGoal {
    fn from(goal: NpcGoal) -> Self {
        match goal {
            NpcGoal::Flee => Self::Survive,
            NpcGoal::Eat | NpcGoal::BuyFood => Self::Eat,
            NpcGoal::Rest => Self::Rest,
            NpcGoal::Work => Self::Work,
            NpcGoal::Patrol => Self::Patrol,
            NpcGoal::Wander => Self::Wander,
            NpcGoal::Idle => Self::Idle,
        }
    }
}

impl GoapGoal {
    /// Checks whether the goal condition is satisfied by the given state.
    #[must_use]
    pub fn is_satisfied(&self, state: &GoapState) -> bool {
        match self {
            Self::Survive => state.is_safe && !state.threat_present,
            Self::Eat => state.hunger_reduced,
            Self::Rest => state.fatigue_reduced,
            Self::Work => state.work_done,
            Self::Patrol | Self::Wander | Self::Idle => true,
        }
    }

    /// Computes admissible heuristic distance (remaining unsatisfied condition count).
    #[must_use]
    pub fn heuristic(&self, state: &GoapState) -> u32 {
        match self {
            Self::Survive => {
                let mut h = 0;
                if !state.is_safe {
                    h += 1;
                }
                if state.threat_present {
                    h += 1;
                }
                h * 100
            }
            Self::Eat => {
                if state.hunger_reduced {
                    0
                } else if state.has_food {
                    200
                } else if state.at_market {
                    300
                } else {
                    350
                }
            }
            Self::Rest => {
                if state.fatigue_reduced {
                    0
                } else if state.near_home {
                    500
                } else {
                    550
                }
            }
            Self::Work => {
                if state.work_done {
                    0
                } else {
                    700
                }
            }
            Self::Patrol | Self::Wander | Self::Idle => 0,
        }
    }
}

/// Action identifiers available to NPCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GoapActionId {
    FleeToSafety = 0,
    MoveToMarket = 1,
    BuyFood = 2,
    EatFood = 3,
    MoveToHome = 4,
    RestAtHome = 5,
    WorkField = 6,
    PatrolArea = 7,
    Idle = 8,
}

impl GoapActionId {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::FleeToSafety => "FleeToSafety",
            Self::MoveToMarket => "MoveToMarket",
            Self::BuyFood => "BuyFood",
            Self::EatFood => "EatFood",
            Self::MoveToHome => "MoveToHome",
            Self::RestAtHome => "RestAtHome",
            Self::WorkField => "WorkField",
            Self::PatrolArea => "PatrolArea",
            Self::Idle => "Idle",
        }
    }

    /// Action execution cost in milli-units.
    #[must_use]
    pub const fn cost(&self) -> u32 {
        match self {
            Self::FleeToSafety => 100,
            Self::MoveToMarket => 50,
            Self::BuyFood => 100,
            Self::EatFood => 200,
            Self::MoveToHome => 50,
            Self::RestAtHome => 500,
            Self::WorkField => 700,
            Self::PatrolArea => 100,
            Self::Idle => 10,
        }
    }

    /// Checks if this action's preconditions are satisfied in the given state.
    #[must_use]
    pub fn preconditions_met(&self, state: &GoapState) -> bool {
        match self {
            Self::FleeToSafety => state.threat_present,
            Self::MoveToMarket => !state.threat_present && !state.at_market,
            Self::BuyFood => {
                state.at_market && state.has_gold && state.shop_open && !state.has_food
            }
            Self::EatFood => state.has_food && !state.in_combat,
            Self::MoveToHome => !state.threat_present && !state.near_home,
            Self::RestAtHome => state.near_home && state.is_safe,
            Self::WorkField => state.is_safe && !state.threat_present,
            Self::PatrolArea => state.is_safe && !state.threat_present,
            Self::Idle => true,
        }
    }

    /// Applies this action's effects to the state, returning the new state.
    #[must_use]
    pub fn apply_effects(&self, state: &GoapState) -> GoapState {
        let mut next = *state;
        match self {
            Self::FleeToSafety => {
                next.threat_present = false;
                next.is_safe = true;
            }
            Self::MoveToMarket => {
                next.at_market = true;
            }
            Self::BuyFood => {
                next.has_food = true;
            }
            Self::EatFood => {
                next.hunger_reduced = true;
                next.has_food = false;
            }
            Self::MoveToHome => {
                next.near_home = true;
            }
            Self::RestAtHome => {
                next.fatigue_reduced = true;
            }
            Self::WorkField => {
                next.work_done = true;
            }
            Self::PatrolArea => {
                next.is_safe = true;
            }
            Self::Idle => {}
        }
        next
    }
}

/// Node in the A* GOAP search space.
#[derive(Clone, Eq, PartialEq)]
struct PlanNode {
    state: GoapState,
    actions: Vec<GoapActionId>,
    g_cost: u32,
    f_cost: u32,
}

impl Ord for PlanNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap
        other
            .f_cost
            .cmp(&self.f_cost)
            .then_with(|| other.g_cost.cmp(&self.g_cost))
            .then_with(|| self.actions.len().cmp(&other.actions.len()))
    }
}

impl PartialOrd for PlanNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// GOAP planner configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoapPlannerConfig {
    pub max_expansions: usize,
    pub max_plan_length: usize,
}

impl Default for GoapPlannerConfig {
    fn default() -> Self {
        Self {
            max_expansions: 128,
            max_plan_length: 12,
        }
    }
}

/// Goal-Oriented Action Planner.
#[derive(Debug)]
pub struct GoapPlanner {
    config: GoapPlannerConfig,
}

impl Default for GoapPlanner {
    fn default() -> Self {
        Self::new(GoapPlannerConfig::default())
    }
}

impl GoapPlanner {
    #[must_use]
    pub const fn new(config: GoapPlannerConfig) -> Self {
        Self { config }
    }

    /// Plans a sequence of actions from `initial_state` to achieve `goal`.
    #[must_use]
    pub fn plan(&self, initial_state: GoapState, goal: GoapGoal) -> Option<Vec<GoapActionId>> {
        if goal.is_satisfied(&initial_state) {
            return Some(Vec::new());
        }

        let mut open_set = BinaryHeap::new();
        let mut visited = HashSet::new();

        let initial_h = goal.heuristic(&initial_state);
        open_set.push(PlanNode {
            state: initial_state,
            actions: Vec::new(),
            g_cost: 0,
            f_cost: initial_h,
        });

        let candidate_actions = [
            GoapActionId::FleeToSafety,
            GoapActionId::MoveToMarket,
            GoapActionId::BuyFood,
            GoapActionId::EatFood,
            GoapActionId::MoveToHome,
            GoapActionId::RestAtHome,
            GoapActionId::WorkField,
            GoapActionId::PatrolArea,
            GoapActionId::Idle,
        ];

        let mut expansions = 0;

        while let Some(current) = open_set.pop() {
            if goal.is_satisfied(&current.state) {
                return Some(current.actions);
            }

            if expansions >= self.config.max_expansions {
                return None;
            }
            expansions += 1;

            if !visited.insert(current.state) {
                continue;
            }

            if current.actions.len() >= self.config.max_plan_length {
                continue;
            }

            for &action in &candidate_actions {
                if action.preconditions_met(&current.state) {
                    let next_state = action.apply_effects(&current.state);
                    if visited.contains(&next_state) {
                        continue;
                    }

                    let mut next_actions = current.actions.clone();
                    next_actions.push(action);

                    let g_cost = current.g_cost.saturating_add(action.cost());
                    let h_cost = goal.heuristic(&next_state);
                    let f_cost = g_cost.saturating_add(h_cost);

                    open_set.push(PlanNode {
                        state: next_state,
                        actions: next_actions,
                        g_cost,
                        f_cost,
                    });
                }
            }
        }

        None
    }
}
