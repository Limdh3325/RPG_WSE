use serde::{Deserialize, Serialize};

/// The canonical 10 execution phases of a deterministic simulation tick.
///
/// Order is strictly enforced and cannot be altered at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TickPhase {
    /// 0. Ingress command dequeue and sorting.
    Admission = 0,
    /// 1. Validate commands against current state invariants; reject invalid.
    CommandValidation = 1,
    /// 2. Spatial awareness and visibility evaluation.
    Perception = 2,
    /// 3. Velocity and coordinate updates with boundary collision checks.
    NavigationAndMovement = 3,
    /// 4. Combat resolution, damage application, death transitions.
    Combat = 4,
    /// 5. Scheduled NPC tactical planning and decision cycles.
    NpcDecisions = 5,
    /// 6. World economy, needs decay (hunger/fatigue), market ticks.
    WorldEconomy = 6,
    /// 7. Event rules evaluation (triggers, environmental hazards).
    EventRules = 7,
    /// 8. Tick increment, canonical state snapshot delta collection.
    Finalize = 8,
    /// 9. Package final tick output, metrics, and durable intents.
    Output = 9,
}

impl TickPhase {
    /// Returns all 10 phases in sequential execution order.
    pub const ALL: [Self; 10] = [
        Self::Admission,
        Self::CommandValidation,
        Self::Perception,
        Self::NavigationAndMovement,
        Self::Combat,
        Self::NpcDecisions,
        Self::WorldEconomy,
        Self::EventRules,
        Self::Finalize,
        Self::Output,
    ];
}
