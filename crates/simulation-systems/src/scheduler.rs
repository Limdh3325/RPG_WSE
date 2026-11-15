//! Budget-based distributed NPC scheduler.
//!
//! Spreads evaluation of large NPC populations (e.g. 1,000 NPCs) across 10Hz ticks,
//! enforcing strict time-slice budgets (`max_npc_evaluations_per_tick`) to prevent frame drops.

use game_domain::EntityId;

/// Configuration for NPC evaluation budgeting and scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcSchedulerConfig {
    /// Nominal interval in ticks between full decisions for each NPC (e.g. 10 ticks = 1 second at 10Hz).
    pub decision_interval_ticks: u64,
    /// Maximum number of NPC evaluations allowed in a single tick.
    pub max_evaluations_per_tick: usize,
}

impl Default for NpcSchedulerConfig {
    fn default() -> Self {
        Self {
            decision_interval_ticks: 10,
            max_evaluations_per_tick: 200,
        }
    }
}

/// Distributed scheduler for NPC evaluations.
pub struct NpcScheduler {
    pub config: NpcSchedulerConfig,
}

impl Default for NpcScheduler {
    fn default() -> Self {
        Self::new(NpcSchedulerConfig::default())
    }
}

impl NpcScheduler {
    #[must_use]
    pub const fn new(config: NpcSchedulerConfig) -> Self {
        Self { config }
    }

    /// Determines whether an entity is deterministically scheduled for evaluation on the given tick.
    #[must_use]
    pub fn is_scheduled(&self, entity_id: EntityId, current_tick: u64) -> bool {
        if self.config.decision_interval_ticks == 0 {
            return true;
        }
        let interval = self.config.decision_interval_ticks as u128;
        let offset = (entity_id.0.as_u128() % interval) as u64;
        (current_tick % self.config.decision_interval_ticks) == offset
    }

    /// Filters and selects the subset of entities to evaluate on this tick,
    /// strictly enforcing the maximum evaluation budget while rotating the selection
    /// window across ticks to prevent NPC starvation under oversubscribed budgets.
    #[must_use]
    pub fn schedule_batch(&self, entity_ids: &[EntityId], current_tick: u64) -> Vec<EntityId> {
        let budget = self.config.max_evaluations_per_tick;
        if budget == 0 || entity_ids.is_empty() {
            return Vec::new();
        }

        let mut eligible: Vec<EntityId> = entity_ids
            .iter()
            .copied()
            .filter(|&id| self.is_scheduled(id, current_tick))
            .collect();

        if eligible.is_empty() {
            return Vec::new();
        }

        // Canonical deterministic sort independent of caller slice or hash order
        eligible.sort_unstable();

        if eligible.len() <= budget {
            return eligible;
        }

        let shard_occurrence = if self.config.decision_interval_ticks == 0 {
            current_tick
        } else {
            current_tick / self.config.decision_interval_ticks
        };

        let start = ((shard_occurrence as usize).wrapping_mul(budget)) % eligible.len();
        let mut scheduled = Vec::with_capacity(budget);
        for i in 0..budget {
            let idx = (start + i) % eligible.len();
            scheduled.push(eligible[idx]);
        }

        scheduled
    }
}
