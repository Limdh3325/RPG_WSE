use game_domain::errors::DomainError;
use game_domain::id::{CommandId, EntityId, EventId, RegionId};
use game_domain::state::NavigationState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default supported event schema version.
pub const SUPPORTED_EVENT_SCHEMA_VERSION: u32 = 1;

/// Discrete events emitted during simulation execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationEvent {
    CommandRejected {
        command_id: CommandId,
        entity_id: EntityId,
        reason: String,
    },
    EntityMoved {
        entity_id: EntityId,
        from_x: i64,
        from_y: i64,
        to_x: i64,
        to_y: i64,
    },
    CombatHit {
        attacker_id: EntityId,
        target_id: EntityId,
        damage: i32,
        remaining_hp: i32,
    },
    EntityDied {
        entity_id: EntityId,
    },
    NpcDecisionExecuted {
        entity_id: EntityId,
        tick: u64,
    },
    FoodShortage {
        settlement_id: Option<EntityId>,
        deficit: i64,
    },
    PurchaseExecuted {
        buyer_id: EntityId,
        item_id: u64,
        price: i64,
    },
    RuleCycleDetected {
        trigger_event: String,
        cycle_depth: usize,
    },
    EventBudgetExceeded {
        tick: u64,
        max_events: usize,
        requested: usize,
    },
    Custom {
        name: String,
        data: String,
    },
    NavigationStateChanged {
        entity_id: EntityId,
        state: NavigationState,
        reason: Option<String>,
    },
    LootIntentProduced {
        killer_id: EntityId,
        victim_id: EntityId,
        item_id: u64,
        amount: u32,
        intent_id: Uuid,
    },
    TickCompleted {
        tick: u64,
        entity_count: usize,
    },
    NpcGoalChanged {
        entity_id: EntityId,
        old_goal: Option<String>,
        new_goal: String,
        score: u32,
        tick: u64,
    },
    NpcPlanFormulated {
        entity_id: EntityId,
        goal: String,
        actions: Vec<String>,
        tick: u64,
    },
    NpcPlanFailed {
        entity_id: EntityId,
        goal: String,
        reason: String,
        consecutive_failures: u32,
        tick: u64,
    },
    NpcActionExecuted {
        entity_id: EntityId,
        action: String,
        tick: u64,
    },
    NpcPlanCompleted {
        entity_id: EntityId,
        goal: String,
        tick: u64,
    },
    NpcMemoryRecorded {
        entity_id: EntityId,
        memory_kind: String,
        source_event_id: Option<Uuid>,
        tick: u64,
    },
    CaravanDispatched {
        caravan_id: String,
        settlement_id: u64,
        food_amount: i64,
    },
    CaravanAmbushed {
        caravan_id: String,
        road_id: String,
        threat_level: u32,
    },
    CaravanResolved {
        caravan_id: String,
        outcome: String,
        food_delivered: i64,
    },
    SettlementSecurityChanged {
        settlement_id: u64,
        new_security_index: i32,
    },
}

impl SimulationEvent {
    /// Canonical event type identifier string.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::CommandRejected { .. } => "command_rejected",
            Self::EntityMoved { .. } => "entity_moved",
            Self::CombatHit { .. } => "combat_hit",
            Self::EntityDied { .. } => "entity_died",
            Self::NpcDecisionExecuted { .. } => "npc_decision_executed",
            Self::FoodShortage { .. } => "food_shortage",
            Self::PurchaseExecuted { .. } => "purchase_executed",
            Self::RuleCycleDetected { .. } => "rule_cycle_detected",
            Self::EventBudgetExceeded { .. } => "event_budget_exceeded",
            Self::Custom { .. } => "custom",
            Self::NavigationStateChanged { .. } => "navigation_state_changed",
            Self::LootIntentProduced { .. } => "loot_intent_produced",
            Self::TickCompleted { .. } => "tick_completed",
            Self::NpcGoalChanged { .. } => "npc_goal_changed",
            Self::NpcPlanFormulated { .. } => "npc_plan_formulated",
            Self::NpcPlanFailed { .. } => "npc_plan_failed",
            Self::NpcActionExecuted { .. } => "npc_action_executed",
            Self::NpcPlanCompleted { .. } => "npc_plan_completed",
            Self::NpcMemoryRecorded { .. } => "npc_memory_recorded",
            Self::CaravanDispatched { .. } => "caravan_dispatched",
            Self::CaravanAmbushed { .. } => "caravan_ambushed",
            Self::CaravanResolved { .. } => "caravan_resolved",
            Self::SettlementSecurityChanged { .. } => "settlement_security_changed",
        }
    }
}

/// Enriched domain event envelope carrying full provenance, causation, and correlation metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub event_type: String,
    pub schema_version: u32,
    pub region_id: RegionId,
    pub tick: u64,
    pub causation_command_id: Option<CommandId>,
    pub causation_event_id: Option<EventId>,
    pub causation_depth: usize,
    pub correlation_id: Uuid,
    pub source_entity_id: Option<EntityId>,
    pub payload: SimulationEvent,
}

impl EventEnvelope {
    /// Creates a deterministic root-level event envelope.
    #[must_use]
    pub fn new_root(
        region_id: RegionId,
        tick: u64,
        event_seq: u64,
        causation_command_id: Option<CommandId>,
        source_entity_id: Option<EntityId>,
        payload: SimulationEvent,
    ) -> Self {
        let event_type = payload.event_type().to_string();
        let event_id = EventId::deterministic(region_id, tick, event_seq);
        let correlation_id = if let Some(cmd_id) = causation_command_id {
            cmd_id.0
        } else {
            event_id.0
        };
        Self {
            event_id,
            event_type,
            schema_version: SUPPORTED_EVENT_SCHEMA_VERSION,
            region_id,
            tick,
            causation_command_id,
            causation_event_id: None,
            causation_depth: 0,
            correlation_id,
            source_entity_id,
            payload,
        }
    }

    /// Creates a deterministic child event envelope inheriting parent correlation ID and incrementing causation depth.
    #[must_use]
    pub fn new_child(
        parent: &Self,
        tick: u64,
        child_index: usize,
        payload: SimulationEvent,
    ) -> Self {
        let event_type = payload.event_type().to_string();
        let event_id = EventId::deterministic_child(parent.event_id, child_index);
        Self {
            event_id,
            event_type,
            schema_version: SUPPORTED_EVENT_SCHEMA_VERSION,
            region_id: parent.region_id,
            tick,
            causation_command_id: parent.causation_command_id,
            causation_event_id: Some(parent.event_id),
            causation_depth: parent.causation_depth + 1,
            correlation_id: parent.correlation_id,
            source_entity_id: parent.source_entity_id,
            payload,
        }
    }
}

/// Event budget limiter preventing unbounded cascade loops and excessive events per tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventBudget {
    pub max_events_per_tick: usize,
    pub max_causation_depth: usize,
    pub current_tick_count: usize,
}

impl EventBudget {
    #[must_use]
    pub const fn new(max_events_per_tick: usize, max_causation_depth: usize) -> Self {
        Self {
            max_events_per_tick,
            max_causation_depth,
            current_tick_count: 0,
        }
    }

    /// Resets the counter for a new simulation tick.
    pub fn reset_tick(&mut self) {
        self.current_tick_count = 0;
    }

    /// Attempts to record an emission against the tick budget.
    pub fn record_emission(&mut self, tick: u64) -> Result<(), DomainError> {
        if self.current_tick_count >= self.max_events_per_tick {
            Err(DomainError::EventBudgetExceeded {
                tick,
                max_events: self.max_events_per_tick,
                requested: self.current_tick_count + 1,
            })
        } else {
            self.current_tick_count += 1;
            Ok(())
        }
    }

    /// Checks whether causation depth is within acceptable limits.
    pub fn check_causation(&self, depth: usize, trigger_name: &str) -> Result<(), DomainError> {
        if depth >= self.max_causation_depth {
            Err(DomainError::RuleCycleDetected {
                trigger_event: trigger_name.to_string(),
                cycle_depth: depth,
            })
        } else {
            Ok(())
        }
    }
}

impl Default for EventBudget {
    fn default() -> Self {
        Self::new(100, 5)
    }
}

/// Action resulting from an event rule evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    /// Emit a subsequent simulation event within the same tick.
    EmitEvent(SimulationEvent),
    /// Schedule a follow-up command for a future tick (delay must be >= 1).
    ScheduleFollowUpCommand {
        delay_ticks: u64,
        command: crate::command::CommandEnvelope,
    },
}

/// Pluggable event rule trait.
pub trait EventRule: Send + Sync {
    fn name(&self) -> &str;
    fn evaluate(&self, event: &EventEnvelope, current_tick: u64) -> Vec<RuleAction>;
}

/// Rule execution engine evaluating declarative rules against emitted events.
#[derive(Default)]
pub struct RuleEngine {
    rules: Vec<Box<dyn EventRule>>,
}

impl RuleEngine {
    #[must_use]
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn register_rule(&mut self, rule: Box<dyn EventRule>) {
        self.rules.push(rule);
    }

    /// Processes an event through all matching rules, recursively cascading child events
    /// while strictly enforcing causation depth limits, event budget, and follow-up scheduling delay.
    pub fn process_event(
        &self,
        event: &EventEnvelope,
        current_tick: u64,
        budget: &mut EventBudget,
    ) -> (Vec<EventEnvelope>, Vec<crate::command::CommandEnvelope>) {
        let mut all_events = Vec::new();
        let mut all_commands = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(event.clone());
        let mut child_counter = 0usize;

        while let Some(current) = queue.pop_front() {
            // Check causation depth limit
            if let Err(DomainError::RuleCycleDetected {
                trigger_event,
                cycle_depth,
            }) = budget.check_causation(current.causation_depth, &current.event_type)
            {
                let cycle_payload = SimulationEvent::RuleCycleDetected {
                    trigger_event,
                    cycle_depth,
                };
                if budget.record_emission(current_tick).is_ok() {
                    let child = EventEnvelope::new_child(
                        &current,
                        current_tick,
                        child_counter,
                        cycle_payload,
                    );
                    child_counter += 1;
                    all_events.push(child);
                }
                continue;
            }

            for rule in &self.rules {
                let actions = rule.evaluate(&current, current_tick);
                for action in actions {
                    match action {
                        RuleAction::EmitEvent(payload) => {
                            if budget.record_emission(current_tick).is_ok() {
                                let child = EventEnvelope::new_child(
                                    &current,
                                    current_tick,
                                    child_counter,
                                    payload,
                                );
                                child_counter += 1;
                                all_events.push(child.clone());
                                queue.push_back(child);
                            } else {
                                let overflow = SimulationEvent::EventBudgetExceeded {
                                    tick: current_tick,
                                    max_events: budget.max_events_per_tick,
                                    requested: budget.current_tick_count + 1,
                                };
                                all_events.push(EventEnvelope::new_child(
                                    &current,
                                    current_tick,
                                    child_counter,
                                    overflow,
                                ));
                                return (all_events, all_commands);
                            }
                        }
                        RuleAction::ScheduleFollowUpCommand {
                            delay_ticks,
                            mut command,
                        } => {
                            let effective_delay = delay_ticks.max(1);
                            command.assigned_tick = current_tick + effective_delay;
                            all_commands.push(command);
                        }
                    }
                }
            }
        }

        (all_events, all_commands)
    }
}
