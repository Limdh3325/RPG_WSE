//! NPC AI Execution System & Lifecycle State Machine.
//!
//! Handles the full lifecycle: `IDLE -> EVALUATING -> PLANNING -> EXECUTING`,
//! re-planning on invalid routes or blocked conditions, and fallback to Idle
//! with cooldown and diagnostic telemetry on consecutive failures (TEST-AI-003).
//! Integrates directly with authoritative `RegionWorld` ECS and Command generation.

use crate::goap::{GoapActionId, GoapGoal, GoapPlanner, GoapPlannerConfig};
use crate::perception::{
    NpcWorldFacts, PerceptionConfig, PerceptionContext, SurroundingEntityData, extract_world_facts,
};
use crate::scheduler::NpcScheduler;
use crate::utility::{NpcGoal, UtilityEvaluator};
use ecs_components::{
    AIState, CombatStats, FactionMembership, MonsterArchetype, Needs, Personality, Position,
    RegionWorld,
};
use game_domain::{CommandId, EntityId, SessionId};
use simulation_core::command::{CommandEnvelope, CommandPayload, CommandStatus};
use simulation_core::event::SimulationEvent;
use std::collections::BTreeMap;

/// High-level lifecycle states for an NPC AI agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NpcAiLifecycleState {
    #[default]
    Idle,
    Evaluating,
    Planning,
    Executing,
    Success,
    Failed,
}

/// Tracks an in-flight command emitted to the authoritative engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcPendingCommand {
    pub command_id: CommandId,
    pub action: GoapActionId,
    pub plan_index: usize,
    pub issued_tick: u64,
}

/// Runtime planning state tracked per NPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcAgentState {
    pub entity_id: EntityId,
    pub lifecycle: NpcAiLifecycleState,
    pub current_goal: Option<NpcGoal>,
    pub current_plan: Vec<GoapActionId>,
    pub step_index: usize,
    pub consecutive_failures: u32,
    pub cooldown_until_tick: u64,
    pub last_plan_tick: u64,
    pub pending_command: Option<NpcPendingCommand>,
    pub superseded_commands: Vec<CommandId>,
}

impl NpcAgentState {
    #[must_use]
    pub const fn new(entity_id: EntityId) -> Self {
        Self {
            entity_id,
            lifecycle: NpcAiLifecycleState::Idle,
            current_goal: None,
            current_plan: Vec::new(),
            step_index: 0,
            consecutive_failures: 0,
            cooldown_until_tick: 0,
            last_plan_tick: 0,
            pending_command: None,
            superseded_commands: Vec::new(),
        }
    }

    pub fn reset_plan(&mut self) {
        self.current_plan.clear();
        self.step_index = 0;
        if let Some(pending) = self.pending_command.take() {
            self.superseded_commands.push(pending.command_id);
        }
    }

    /// Reconciles an authoritative engine command execution outcome.
    pub fn reconcile_command_outcome(
        &mut self,
        command_id: CommandId,
        succeeded: bool,
        tick: u64,
        events: &mut Vec<SimulationEvent>,
    ) {
        // Discard outcomes from superseded commands that were cancelled by urgent threat interrupts
        if self.superseded_commands.contains(&command_id) {
            self.superseded_commands.retain(|id| *id != command_id);
            return;
        }

        let Some(pending) = self.pending_command else {
            return;
        };
        if pending.command_id != command_id {
            return;
        }

        self.pending_command = None;
        if succeeded {
            self.consecutive_failures = 0;
            self.step_index += 1;
            if self.step_index >= self.current_plan.len() {
                self.lifecycle = NpcAiLifecycleState::Success;
                let goal_name = self
                    .current_goal
                    .map_or_else(|| "Unknown".to_string(), |g| g.name().to_string());
                events.push(SimulationEvent::NpcPlanCompleted {
                    entity_id: self.entity_id,
                    goal: goal_name,
                    tick,
                });
            } else {
                self.lifecycle = NpcAiLifecycleState::Executing;
            }
        } else {
            self.consecutive_failures += 1;
            let goal_name = self
                .current_goal
                .map_or_else(|| "Unknown".to_string(), |g| g.name().to_string());
            events.push(SimulationEvent::NpcPlanFailed {
                entity_id: self.entity_id,
                goal: goal_name,
                reason: "Command execution failed or rejected by engine".to_string(),
                consecutive_failures: self.consecutive_failures,
                tick,
            });
            self.reset_plan();
            if self.consecutive_failures >= 3 {
                self.lifecycle = NpcAiLifecycleState::Idle;
                self.cooldown_until_tick = tick + 30;
            } else {
                self.lifecycle = NpcAiLifecycleState::Planning;
            }
        }
    }
}

/// Complete NPC AI decision-making and execution system.
#[derive(Debug)]
pub struct NpcAiSystem {
    pub perception_config: PerceptionConfig,
    pub utility_evaluator: UtilityEvaluator,
    pub planner: GoapPlanner,
}

impl Default for NpcAiSystem {
    fn default() -> Self {
        Self {
            perception_config: PerceptionConfig::default(),
            utility_evaluator: UtilityEvaluator::default(),
            planner: GoapPlanner::new(GoapPlannerConfig::default()),
        }
    }
}

impl NpcAiSystem {
    #[must_use]
    pub fn new(
        perception_config: PerceptionConfig,
        utility_evaluator: UtilityEvaluator,
        planner_config: GoapPlannerConfig,
    ) -> Self {
        Self {
            perception_config,
            utility_evaluator,
            planner: GoapPlanner::new(planner_config),
        }
    }

    /// Executes an AI decision cycle for an individual NPC agent.
    ///
    /// Applies authoritative modifications to `Needs` upon action execution,
    /// returns generated simulation events, and produces an optional `CommandEnvelope` for the engine.
    #[allow(clippy::too_many_arguments)]
    pub fn step_npc(
        &self,
        agent: &mut NpcAgentState,
        ctx: &PerceptionContext<'_>,
        surrounding_entities: &[SurroundingEntityData],
        needs: &Needs,
        personality: &Personality,
        current_tick: u64,
        events: &mut Vec<SimulationEvent>,
    ) -> (Option<AIState>, Option<CommandEnvelope>) {
        // 1. Sensory Perception (Urgent Reaction check)
        let facts = extract_world_facts(ctx, surrounding_entities, &self.perception_config);

        // Interrupt non-fleeing plan immediately if imminent threat is perceived (TEST-AI-001)
        if facts.threat_present && agent.current_goal != Some(NpcGoal::Flee) {
            agent.reset_plan();
            agent.pending_command = None;
            agent.current_goal = Some(NpcGoal::Flee);
            agent.lifecycle = NpcAiLifecycleState::Planning;
        }

        // 2. Awaiting pending command outcome from authoritative engine (if not urgently threatened)
        if let Some(pending) = agent.pending_command {
            return (
                Some(AIState {
                    goal: agent
                        .current_goal
                        .map_or_else(|| "Idle".to_string(), |g| g.name().to_string()),
                    action: format!("Awaiting_{:?}", pending.action),
                    next_decision_tick: current_tick + 1,
                }),
                None,
            );
        }

        // 3. Cooldown check
        if current_tick < agent.cooldown_until_tick {
            agent.lifecycle = NpcAiLifecycleState::Idle;
            return (
                Some(AIState {
                    goal: agent
                        .current_goal
                        .map_or_else(|| "Idle".to_string(), |g| g.name().to_string()),
                    action: "Idle_Cooldown".to_string(),
                    next_decision_tick: agent.cooldown_until_tick,
                }),
                None,
            );
        }

        // 4. Goal Selection
        let (selected_goal, goal_score) = self.utility_evaluator.select_goal(
            agent.current_goal,
            &facts,
            needs,
            personality,
            agent.entity_id,
        );

        if agent.current_goal != Some(selected_goal) {
            events.push(SimulationEvent::NpcGoalChanged {
                entity_id: agent.entity_id,
                old_goal: agent.current_goal.map(|g| g.name().to_string()),
                new_goal: selected_goal.name().to_string(),
                score: goal_score,
                tick: current_tick,
            });
            agent.current_goal = Some(selected_goal);
            agent.reset_plan();
            agent.lifecycle = NpcAiLifecycleState::Planning;
        }

        // Interrupt non-fleeing plan immediately if danger arises
        if facts.threat_present
            && selected_goal == NpcGoal::Flee
            && agent.current_goal != Some(NpcGoal::Flee)
        {
            agent.current_goal = Some(NpcGoal::Flee);
            agent.reset_plan();
            agent.lifecycle = NpcAiLifecycleState::Planning;
        }

        // 5. Planning
        if agent.current_plan.is_empty() || agent.step_index >= agent.current_plan.len() {
            agent.lifecycle = NpcAiLifecycleState::Planning;
            let goap_goal: GoapGoal = selected_goal.into();
            let goap_state = (&facts).into();

            if let Some(plan) = self.planner.plan(goap_state, goap_goal) {
                events.push(SimulationEvent::NpcPlanFormulated {
                    entity_id: agent.entity_id,
                    goal: selected_goal.name().to_string(),
                    actions: plan.iter().map(|a| a.name().to_string()).collect(),
                    tick: current_tick,
                });
                agent.current_plan = plan;
                agent.step_index = 0;
                agent.last_plan_tick = current_tick;
                agent.lifecycle = NpcAiLifecycleState::Executing;
            } else {
                // Planner failed to find a valid route/chain
                agent.consecutive_failures += 1;
                events.push(SimulationEvent::NpcPlanFailed {
                    entity_id: agent.entity_id,
                    goal: selected_goal.name().to_string(),
                    reason: "No valid action plan found".to_string(),
                    consecutive_failures: agent.consecutive_failures,
                    tick: current_tick,
                });

                // Fallback to Idle on 3 consecutive failures (TEST-AI-003)
                if agent.consecutive_failures >= 3 {
                    agent.lifecycle = NpcAiLifecycleState::Idle;
                    agent.cooldown_until_tick = current_tick + 30;
                    agent.reset_plan();
                }

                return (
                    Some(AIState {
                        goal: selected_goal.name().to_string(),
                        action: "Planning_Failed".to_string(),
                        next_decision_tick: current_tick + 1,
                    }),
                    None,
                );
            }
        }

        // 6. Action Execution Step
        if agent.step_index < agent.current_plan.len() {
            let action = agent.current_plan[agent.step_index];
            let goap_state = (&facts).into();

            // Validate preconditions before execution (e.g. invalid route or blocked prerequisite)
            if !action.preconditions_met(&goap_state) {
                agent.consecutive_failures += 1;
                events.push(SimulationEvent::NpcPlanFailed {
                    entity_id: agent.entity_id,
                    goal: selected_goal.name().to_string(),
                    reason: format!("Precondition failed for action {}", action.name()),
                    consecutive_failures: agent.consecutive_failures,
                    tick: current_tick,
                });

                if agent.consecutive_failures >= 3 {
                    // Fallback to Idle with cooldown (TEST-AI-003)
                    agent.lifecycle = NpcAiLifecycleState::Idle;
                    agent.cooldown_until_tick = current_tick + 30;
                    agent.reset_plan();
                } else {
                    // Re-plan next tick
                    agent.lifecycle = NpcAiLifecycleState::Planning;
                    agent.reset_plan();
                }

                return (
                    Some(AIState {
                        goal: selected_goal.name().to_string(),
                        action: format!("{}_PreconditionFailed", action.name()),
                        next_decision_tick: current_tick + 1,
                    }),
                    None,
                );
            }

            // Execute action and emit notification event
            events.push(SimulationEvent::NpcActionExecuted {
                entity_id: agent.entity_id,
                action: action.name().to_string(),
                tick: current_tick,
            });

            // Synthesize executable CommandEnvelope for the engine with deterministic CommandId and authoritative region
            let cmd_id = CommandId::from_u128(
                agent.entity_id.0.as_u128()
                    ^ (current_tick as u128)
                    ^ ((agent.step_index as u128) << 64),
            );

            let command = match action {
                GoapActionId::MoveToMarket => {
                    let target = ctx.market_pos.unwrap_or(Position {
                        x_millitile: 5000,
                        y_millitile: 5000,
                    });
                    Some(CommandEnvelope::new(
                        cmd_id,
                        agent.entity_id,
                        ctx.region_id,
                        ctx.session_id,
                        current_tick,
                        current_tick + 1,
                        current_tick,
                        1,
                        CommandPayload::Move {
                            target_x: target.x_millitile,
                            target_y: target.y_millitile,
                        },
                    ))
                }
                GoapActionId::MoveToHome => {
                    let target = ctx.home_pos.unwrap_or(Position {
                        x_millitile: 0,
                        y_millitile: 0,
                    });
                    Some(CommandEnvelope::new(
                        cmd_id,
                        agent.entity_id,
                        ctx.region_id,
                        ctx.session_id,
                        current_tick,
                        current_tick + 1,
                        current_tick,
                        1,
                        CommandPayload::Move {
                            target_x: target.x_millitile,
                            target_y: target.y_millitile,
                        },
                    ))
                }
                GoapActionId::FleeToSafety => {
                    let flee_target = if let Some(threat_pos) = facts.threat_pos {
                        let dx = ctx.self_pos.x_millitile - threat_pos.x_millitile;
                        let dy = ctx.self_pos.y_millitile - threat_pos.y_millitile;
                        let step_x = if dx >= 0 { 2000 } else { -2000 };
                        let step_y = if dy >= 0 { 2000 } else { -2000 };
                        Position {
                            x_millitile: (ctx.self_pos.x_millitile + step_x).clamp(0, 100_000),
                            y_millitile: (ctx.self_pos.y_millitile + step_y).clamp(0, 100_000),
                        }
                    } else {
                        Position {
                            x_millitile: ctx.self_pos.x_millitile.saturating_add(2000),
                            y_millitile: ctx.self_pos.y_millitile.saturating_add(2000),
                        }
                    };
                    Some(CommandEnvelope::new(
                        cmd_id,
                        agent.entity_id,
                        ctx.region_id,
                        ctx.session_id,
                        current_tick,
                        current_tick + 1,
                        current_tick,
                        1,
                        CommandPayload::Move {
                            target_x: flee_target.x_millitile,
                            target_y: flee_target.y_millitile,
                        },
                    ))
                }
                GoapActionId::BuyFood => Some(CommandEnvelope::new(
                    cmd_id,
                    agent.entity_id,
                    ctx.region_id,
                    ctx.session_id,
                    current_tick,
                    current_tick + 1,
                    current_tick,
                    1,
                    CommandPayload::Purchase {
                        item_id: 1001,
                        price: 10,
                    },
                )),
                GoapActionId::EatFood => Some(CommandEnvelope::new(
                    cmd_id,
                    agent.entity_id,
                    ctx.region_id,
                    ctx.session_id,
                    current_tick,
                    current_tick + 1,
                    current_tick,
                    1,
                    CommandPayload::Custom {
                        action: "npc_eat_food".to_string(),
                        payload: Vec::new(),
                    },
                )),
                GoapActionId::RestAtHome => Some(CommandEnvelope::new(
                    cmd_id,
                    agent.entity_id,
                    ctx.region_id,
                    ctx.session_id,
                    current_tick,
                    current_tick + 1,
                    current_tick,
                    1,
                    CommandPayload::Custom {
                        action: "npc_rest".to_string(),
                        payload: Vec::new(),
                    },
                )),
                GoapActionId::WorkField => Some(CommandEnvelope::new(
                    cmd_id,
                    agent.entity_id,
                    ctx.region_id,
                    ctx.session_id,
                    current_tick,
                    current_tick + 1,
                    current_tick,
                    1,
                    CommandPayload::Custom {
                        action: "npc_work".to_string(),
                        payload: Vec::new(),
                    },
                )),
                _ => None,
            };

            if let Some(ref cmd) = command {
                agent.pending_command = Some(NpcPendingCommand {
                    command_id: cmd.command_id,
                    action,
                    plan_index: agent.step_index,
                    issued_tick: current_tick,
                });
            } else {
                agent.consecutive_failures = 0;
                agent.step_index += 1;
                if agent.step_index >= agent.current_plan.len() {
                    agent.lifecycle = NpcAiLifecycleState::Success;
                } else {
                    agent.lifecycle = NpcAiLifecycleState::Executing;
                }
            }

            (
                Some(AIState {
                    goal: selected_goal.name().to_string(),
                    action: action.name().to_string(),
                    next_decision_tick: current_tick + 1,
                }),
                command,
            )
        } else {
            agent.lifecycle = NpcAiLifecycleState::Success;
            agent.reset_plan();
            (
                Some(AIState {
                    goal: selected_goal.name().to_string(),
                    action: "Completed".to_string(),
                    next_decision_tick: current_tick + 10,
                }),
                None,
            )
        }
    }

    /// Steps NPC decision making across an authoritative `RegionWorld`,
    /// directly updating `Needs`, `AIState`, and `Position` in ECS,
    /// while enqueuing generated commands and events.
    #[allow(clippy::too_many_arguments)]
    pub fn step_simulation_world(
        &self,
        world: &mut RegionWorld,
        scheduler: &NpcScheduler,
        agents: &mut BTreeMap<EntityId, NpcAgentState>,
        current_tick: u64,
        session_id: SessionId,
        events: &mut Vec<SimulationEvent>,
        commands: &mut Vec<CommandEnvelope>,
        command_outcomes: &BTreeMap<CommandId, CommandStatus>,
    ) {
        // Collect all entity IDs with Personality in the world
        let npc_ids: Vec<EntityId> = world.index.iter().map(|(&id, _)| id).collect();

        // Filter those scheduled under the tick budget
        let scheduled = scheduler.schedule_batch(&npc_ids, current_tick);

        // First pass: collect entity spatial data for surrounding queries
        let mut entity_data = Vec::new();
        for &id in &npc_ids {
            if let Some(entity) = world.get_entity(id) {
                let entity_ref = world.world.entity(entity);
                let pos = entity_ref.get::<Position>().copied().unwrap_or_default();
                let faction = entity_ref.get::<FactionMembership>().copied();
                let combat = entity_ref.get::<CombatStats>().copied();
                let monster = entity_ref.get::<MonsterArchetype>().copied();
                entity_data.push((id, pos, faction, combat, monster));
            }
        }

        // First pass: Reconcile in-flight command outcomes for all agents awaiting engine results
        for agent in agents.values_mut() {
            if let Some(pending) = agent.pending_command {
                if let Some(&status) = command_outcomes.get(&pending.command_id) {
                    match status {
                        CommandStatus::Succeeded => {
                            agent.reconcile_command_outcome(
                                pending.command_id,
                                true,
                                current_tick,
                                events,
                            );
                        }
                        CommandStatus::Failed | CommandStatus::Rejected => {
                            agent.reconcile_command_outcome(
                                pending.command_id,
                                false,
                                current_tick,
                                events,
                            );
                        }
                        CommandStatus::Received
                        | CommandStatus::Accepted
                        | CommandStatus::PendingCommit => {
                            // Retain pending command while in-flight in engine
                        }
                    }
                } else if current_tick > pending.issued_tick + 10 {
                    // Timeout after 10 ticks without engine acknowledgment
                    agent.reconcile_command_outcome(
                        pending.command_id,
                        false,
                        current_tick,
                        events,
                    );
                }
            }
        }

        for id in scheduled {
            let Some(bevy_entity) = world.get_entity(id) else {
                continue;
            };

            let agent = agents.entry(id).or_insert_with(|| NpcAgentState::new(id));

            // Extract entity components
            let entity_mut = world.world.entity_mut(bevy_entity);
            let Some(personality) = entity_mut.get::<Personality>().copied() else {
                continue;
            };
            let needs = entity_mut.get::<Needs>().copied().unwrap_or_default();
            let pos = entity_mut.get::<Position>().copied().unwrap_or_default();
            let faction = entity_mut.get::<FactionMembership>().copied();
            let inv = entity_mut
                .get::<ecs_components::InventoryProjection>()
                .cloned();

            let ctx = PerceptionContext {
                self_entity_id: id,
                self_pos: pos,
                self_faction: faction.map(|f| game_domain::FactionId(f.faction_id)),
                inventory: inv.as_ref(),
                market_pos: Some(Position {
                    x_millitile: 5000,
                    y_millitile: 5000,
                }),
                home_pos: Some(Position {
                    x_millitile: 0,
                    y_millitile: 0,
                }),
                region_id: world.meta.region_id,
                session_id,
            };

            let surrounding: Vec<SurroundingEntityData> = entity_data
                .iter()
                .filter(|(other_id, _, _, _, _)| *other_id != id)
                .copied()
                .collect();

            let (ai_state, command) = self.step_npc(
                agent,
                &ctx,
                &surrounding,
                &needs,
                &personality,
                current_tick,
                events,
            );

            // Update AIState component in ECS world for telemetry and inspection
            let mut entity_mut_back = world.world.entity_mut(bevy_entity);
            if let Some(new_ai) = ai_state {
                if let Some(mut existing_ai) = entity_mut_back.get_mut::<AIState>() {
                    *existing_ai = new_ai;
                } else {
                    entity_mut_back.insert(new_ai);
                }
            }

            if let Some(cmd) = command {
                commands.push(cmd);
            }
        }
    }
}

impl From<NpcWorldFacts> for crate::goap::GoapState {
    fn from(facts: NpcWorldFacts) -> Self {
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

impl From<crate::goap::GoapState> for NpcWorldFacts {
    fn from(s: crate::goap::GoapState) -> Self {
        Self {
            threat_present: s.threat_present,
            threat_level: if s.threat_present { 500 } else { 0 },
            threat_entity_id: None,
            threat_pos: None,
            is_safe: s.is_safe,
            in_combat: s.in_combat,
            has_food: s.has_food,
            has_gold: s.has_gold,
            shop_open: s.shop_open,
            at_market: s.at_market,
            near_home: s.near_home,
            hunger_reduced: s.hunger_reduced,
            fatigue_reduced: s.fatigue_reduced,
            work_done: s.work_done,
        }
    }
}
