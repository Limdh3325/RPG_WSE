use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use ecs_components::{
    components::{
        CombatStats, Health, InventoryProjection, MonsterArchetype, Navigation, Needs, Personality,
        Position, StableId, Velocity,
    },
    region_world::RegionWorld,
};
use game_domain::{
    errors::DomainError,
    id::EntityId,
    state::{CombatStatus, HealthDto, NavigationState, PositionDto, VelocityDto},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    clock::{ClockStatus, SimulationClock},
    command::{
        CommandEnvelope, CommandPayload, CommandQueue, CommandStatus, DeduplicationOutcome,
        IngressCommand, TransientCommandDeduplicator,
    },
    event::{EventBudget, EventEnvelope, EventRule, RuleEngine, SimulationEvent},
    phase::TickPhase,
    rng::DeterministicRng,
};

/// Durable intent record for asynchronous database settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableLootIntent {
    pub intent_id: Uuid,
    pub killer_id: EntityId,
    pub victim_id: EntityId,
    pub item_id: u64,
    pub amount: u32,
    pub tick: u64,
}

/// Output produced by a single simulation tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickOutput {
    pub tick: u64,
    pub state_hash: String,
    pub events: Vec<SimulationEvent>,
    pub event_envelopes: Vec<EventEnvelope>,
    pub clock_status: ClockStatus,
    pub executed_npc_decisions: usize,
    pub durable_loot_intents: Vec<DurableLootIntent>,
}

/// Summary report after executing a batch of headless simulation ticks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationSummary {
    pub seed: u64,
    pub start_tick: u64,
    pub end_tick: u64,
    pub total_ticks_executed: u64,
    pub final_state_hash: String,
    pub total_events_emitted: usize,
    pub clock_status: ClockStatus,
    pub npc_decision_counts: BTreeMap<String, u64>,
}

/// Deterministic, single-threaded simulation runtime engine for a Region.
pub struct SimulationEngine {
    pub world: RegionWorld,
    pub clock: SimulationClock,
    pub rng: DeterministicRng,
    pub command_queue: CommandQueue,
    pub deduplicator: TransientCommandDeduplicator,
    pub event_budget: EventBudget,
    pub rule_engine: RuleEngine,
    pub npc_decision_interval: u64,
    pub npc_decision_counts: BTreeMap<EntityId, u64>,
    pub pending_loot_intents: Vec<DurableLootIntent>,
    pub produced_loot_victims: BTreeSet<EntityId>,
    pub settled_loot_intents: BTreeSet<Uuid>,
}

impl SimulationEngine {
    /// Creates a new SimulationEngine for a region world with a deterministic seed.
    #[must_use]
    pub fn new(world: RegionWorld, seed: u64) -> Self {
        Self {
            world,
            clock: SimulationClock::default(),
            rng: DeterministicRng::new(seed),
            command_queue: CommandQueue::new(),
            deduplicator: TransientCommandDeduplicator::default(),
            event_budget: EventBudget::default(),
            rule_engine: RuleEngine::new(),
            npc_decision_interval: 5,
            npc_decision_counts: BTreeMap::new(),
            pending_loot_intents: Vec::new(),
            produced_loot_victims: BTreeSet::new(),
            settled_loot_intents: BTreeSet::new(),
        }
    }

    /// Acknowledges and settles a durable loot intent, removing it from pending and recording it as settled.
    pub fn acknowledge_loot_intent(&mut self, intent_id: Uuid) -> bool {
        if let Some(pos) = self
            .pending_loot_intents
            .iter()
            .position(|i| i.intent_id == intent_id)
        {
            self.pending_loot_intents.remove(pos);
            self.settled_loot_intents.insert(intent_id);
            true
        } else {
            false
        }
    }

    /// Checks if a durable loot intent has already been settled.
    #[must_use]
    pub fn is_loot_intent_settled(&self, intent_id: &Uuid) -> bool {
        self.settled_loot_intents.contains(intent_id)
    }

    /// Registers a custom event rule into the simulation engine.
    pub fn register_rule(&mut self, rule: Box<dyn EventRule>) {
        self.rule_engine.register_rule(rule);
    }

    /// Submits a high-level command envelope, applying schema validation and transient deduplication.
    pub fn submit_envelope(
        &mut self,
        envelope: CommandEnvelope,
    ) -> Result<DeduplicationOutcome, DomainError> {
        let outcome = self.deduplicator.check_and_register(&envelope)?;
        if outcome == DeduplicationOutcome::New {
            self.command_queue.enqueue(envelope.to_ingress());
        }
        Ok(outcome)
    }

    /// Submits an ingress command to be scheduled and executed at its `assigned_tick`.
    ///
    /// Contracts (schema version, payload hash integrity, transient deduplication,
    /// and idempotency conflict checking) are strictly enforced by converting the command
    /// into a canonical CommandEnvelope for this region before enqueueing.
    pub fn submit_command(
        &mut self,
        cmd: IngressCommand,
    ) -> Result<DeduplicationOutcome, DomainError> {
        let envelope = CommandEnvelope::from_ingress(&cmd, self.world.meta.region_id);
        self.submit_envelope(envelope)
    }

    /// Returns the current deduplication and execution status of a command.
    #[must_use]
    pub fn get_command_status(&self, command_id: game_domain::CommandId) -> Option<CommandStatus> {
        self.deduplicator.get_status(command_id)
    }

    /// Spawns a basic actor entity with Position, Velocity, Health, and Personality components.
    pub fn spawn_actor(
        &mut self,
        id: EntityId,
        x_millitile: i64,
        y_millitile: i64,
        max_health: i32,
    ) -> Result<(), DomainError> {
        let pos = PositionDto {
            x_millitile,
            y_millitile,
        };
        let vel = VelocityDto { dx: 0, dy: 0 };
        let health = HealthDto {
            current: max_health,
            max: max_health,
            status: CombatStatus::Alive,
        };
        self.world
            .spawn_entity(game_domain::state::EntityStateDto {
                id,
                position: pos,
                velocity: Some(vel),
                health: Some(health),
                needs: None,
                personality: None,
                inventory: None,
                faction: None,
                collider: None,
            })?;

        if let Some(entity) = self.world.get_entity(id) {
            let mut entity_mut = self.world.world.entity_mut(entity);
            if entity_mut.get::<CombatStats>().is_none() {
                entity_mut.insert(CombatStats::new(1500, 10, 0, 3));
            }
            if entity_mut.get::<Navigation>().is_none() {
                entity_mut.insert(Navigation::new(200));
            }
        }
        Ok(())
    }

    /// Spawns a monster entity with Position, Health, CombatStats, Navigation, and MonsterArchetype.
    pub fn spawn_monster(
        &mut self,
        archetype: MonsterArchetype,
        x_millitile: i64,
        y_millitile: i64,
    ) -> Result<EntityId, DomainError> {
        let entity_id = EntityId::new();
        let max_hp = archetype.max_health();
        let combat_stats = archetype.combat_stats();
        let speed = archetype.movement_speed_millitiles_per_tick();

        let entity = self
            .world
            .world
            .spawn((
                StableId(entity_id),
                Position::new(x_millitile, y_millitile),
                Velocity::new(0, 0),
                Health::new(max_hp, max_hp)?,
                combat_stats,
                archetype,
                Navigation::new(speed),
            ))
            .id();
        self.world.index.insert(entity_id, entity)?;
        Ok(entity_id)
    }

    /// Advances the simulation by exactly one tick, tracking telemetry for clock degradation.
    pub fn step(&mut self) -> Result<TickOutput, DomainError> {
        let start = Instant::now();
        let output = self.step_core()?;
        self.record_tick_telemetry(start.elapsed());
        Ok(output)
    }

    /// Advances the simulation by exactly one tick, injecting explicit telemetry for deterministic testing.
    pub fn step_with_telemetry(
        &mut self,
        elapsed: std::time::Duration,
    ) -> Result<TickOutput, DomainError> {
        let output = self.step_core()?;
        self.record_tick_telemetry(elapsed);
        Ok(output)
    }

    /// Records tick execution telemetry against the simulation clock's fixed tick delta.
    pub fn record_tick_telemetry(&mut self, elapsed: std::time::Duration) {
        if (elapsed.as_millis() as u64) > u64::from(self.clock.fixed_delta_ms) {
            self.clock.record_overrun();
        } else {
            self.clock.record_on_time();
        }
    }

    /// Executes the 10 canonical simulation phases sequentially.
    fn step_core(&mut self) -> Result<TickOutput, DomainError> {
        let current_tick = self.clock.advance_tick();
        self.world.meta.tick = current_tick;
        self.event_budget.reset_tick();
        let mut events = Vec::new();
        let mut budget_overflow_reported = false;

        let mut emit =
            |event: SimulationEvent, budget: &mut EventBudget, evs: &mut Vec<SimulationEvent>| {
                if budget.max_events_per_tick == 0 {
                    return;
                }
                if budget.record_emission(current_tick).is_ok() {
                    evs.push(event);
                } else if !budget_overflow_reported {
                    budget_overflow_reported = true;
                    if evs.len() >= budget.max_events_per_tick && !evs.is_empty() {
                        evs.pop();
                    }
                    if evs.len() < budget.max_events_per_tick {
                        evs.push(SimulationEvent::EventBudgetExceeded {
                            tick: current_tick,
                            max_events: budget.max_events_per_tick,
                            requested: budget.current_tick_count,
                        });
                    }
                }
            };

        // 0. Admission Phase: Drain sorted ingress commands for this tick
        let ingress_commands = self.command_queue.drain_for_tick(current_tick);

        // 1. Command Validation Phase: Validate invariants, filter valid intents
        let mut validated_moves = Vec::new();
        let mut validated_attacks = Vec::new();
        let mut tick_attackers = BTreeSet::new();

        for cmd in ingress_commands {
            // Transition status to PendingCommit if command is tracked in deduplicator
            let _ = self
                .deduplicator
                .update_status(cmd.command_id, CommandStatus::PendingCommit);

            let entity_opt = self.world.get_entity(cmd.entity_id);
            if entity_opt.is_none() {
                emit(
                    SimulationEvent::CommandRejected {
                        command_id: cmd.command_id,
                        entity_id: cmd.entity_id,
                        reason: "entity not found".to_string(),
                    },
                    &mut self.event_budget,
                    &mut events,
                );
                let _ = self
                    .deduplicator
                    .update_status(cmd.command_id, CommandStatus::Failed);
                continue;
            }

            let _entity = entity_opt.unwrap();
            if matches!(self.world.get_health(cmd.entity_id), Ok(health) if health.status == CombatStatus::Dead)
            {
                emit(
                    SimulationEvent::CommandRejected {
                        command_id: cmd.command_id,
                        entity_id: cmd.entity_id,
                        reason: "entity is dead".to_string(),
                    },
                    &mut self.event_budget,
                    &mut events,
                );
                let _ = self
                    .deduplicator
                    .update_status(cmd.command_id, CommandStatus::Failed);
                continue;
            }

            match cmd.payload {
                CommandPayload::Move { target_x, target_y } => {
                    if !self.world.grid.is_in_bounds_millitile(target_x, target_y) {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: "target out of bounds".to_string(),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                    } else if !self.world.grid.is_millitile_walkable(target_x, target_y) {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: "target position is not walkable".to_string(),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        if let Some(entity_ref) = self.world.get_entity(cmd.entity_id) {
                            let mut entity_mut = self.world.world.entity_mut(entity_ref);
                            if let Some(mut nav) = entity_mut.get_mut::<Navigation>() {
                                nav.state = NavigationState::Blocked;
                                nav.failure_reason =
                                    Some("target position is not walkable".to_string());
                            }
                        }
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                    } else {
                        validated_moves.push((cmd.entity_id, target_x, target_y));
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Succeeded);
                    }
                }
                CommandPayload::SetVelocity { dx, dy } => {
                    const MAX_SPEED: i32 = 500;
                    let dx_abs = (dx as i128).unsigned_abs();
                    let dy_abs = (dy as i128).unsigned_abs();
                    let speed_sq = (dx_abs * dx_abs).saturating_add(dy_abs * dy_abs);
                    let max_speed_sq = (MAX_SPEED as u128) * (MAX_SPEED as u128);
                    if speed_sq > max_speed_sq {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: format!(
                                    "speed exceeds maximum authority ({MAX_SPEED} millitiles/tick)"
                                ),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                        continue;
                    }

                    if let Some(entity_ref) = self.world.get_entity(cmd.entity_id) {
                        let mut entity_mut = self.world.world.entity_mut(entity_ref);
                        if let Some(mut vel) = entity_mut.get_mut::<Velocity>() {
                            vel.dx = dx;
                            vel.dy = dy;
                        }
                    }
                    let _ = self
                        .deduplicator
                        .update_status(cmd.command_id, CommandStatus::Succeeded);
                }
                CommandPayload::Attack { target_id } => {
                    if tick_attackers.contains(&cmd.entity_id) {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: "attack on cooldown (duplicate attack in tick)".to_string(),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                        continue;
                    }

                    let attacker_check =
                        if let Some(attacker_entity) = self.world.get_entity(cmd.entity_id) {
                            let attacker_ref = self.world.world.entity(attacker_entity);
                            let alive = attacker_ref
                                .get::<Health>()
                                .map(|h| h.status == CombatStatus::Alive)
                                .unwrap_or(true);
                            let pos = attacker_ref.get::<Position>().copied();
                            let stats = attacker_ref.get::<CombatStats>().copied();
                            Some((alive, pos, stats))
                        } else {
                            None
                        };

                    let target_check = if let Some(target_entity) = self.world.get_entity(target_id)
                    {
                        let target_ref = self.world.world.entity(target_entity);
                        let alive = target_ref
                            .get::<Health>()
                            .map(|h| h.status == CombatStatus::Alive)
                            .unwrap_or(true);
                        let pos = target_ref.get::<Position>().copied();
                        Some((alive, pos))
                    } else {
                        None
                    };

                    match (attacker_check, target_check) {
                        (None, _) => {
                            emit(
                                SimulationEvent::CommandRejected {
                                    command_id: cmd.command_id,
                                    entity_id: cmd.entity_id,
                                    reason: "attacker entity not found".to_string(),
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Failed);
                        }
                        (_, None) => {
                            emit(
                                SimulationEvent::CommandRejected {
                                    command_id: cmd.command_id,
                                    entity_id: cmd.entity_id,
                                    reason: "target entity not found".to_string(),
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Failed);
                        }
                        (Some((false, _, _)), _) => {
                            emit(
                                SimulationEvent::CommandRejected {
                                    command_id: cmd.command_id,
                                    entity_id: cmd.entity_id,
                                    reason: "attacker is not alive".to_string(),
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Failed);
                        }
                        (_, Some((false, _))) => {
                            emit(
                                SimulationEvent::CommandRejected {
                                    command_id: cmd.command_id,
                                    entity_id: cmd.entity_id,
                                    reason: "target is not alive".to_string(),
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Failed);
                        }
                        (Some((true, atk_pos, atk_stats)), Some((true, tgt_pos))) => {
                            if matches!(atk_stats, Some(stats) if !stats.can_attack(current_tick)) {
                                emit(
                                    SimulationEvent::CommandRejected {
                                        command_id: cmd.command_id,
                                        entity_id: cmd.entity_id,
                                        reason: "attack on cooldown".to_string(),
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                                let _ = self
                                    .deduplicator
                                    .update_status(cmd.command_id, CommandStatus::Failed);
                                continue;
                            }

                            if let (Some(a_pos), Some(t_pos), Some(stats)) =
                                (atk_pos, tgt_pos, atk_stats)
                            {
                                let dx = (a_pos.x_millitile as i128 - t_pos.x_millitile as i128)
                                    .unsigned_abs();
                                let dy = (a_pos.y_millitile as i128 - t_pos.y_millitile as i128)
                                    .unsigned_abs();
                                let dist_sq = (dx * dx).saturating_add(dy * dy);
                                let range = stats.attack_range_millitile.max(0) as u128;
                                let range_sq = range.saturating_mul(range);
                                if dist_sq > range_sq {
                                    emit(
                                        SimulationEvent::CommandRejected {
                                            command_id: cmd.command_id,
                                            entity_id: cmd.entity_id,
                                            reason: "target out of range".to_string(),
                                        },
                                        &mut self.event_budget,
                                        &mut events,
                                    );
                                    let _ = self
                                        .deduplicator
                                        .update_status(cmd.command_id, CommandStatus::Failed);
                                    continue;
                                }
                            }

                            tick_attackers.insert(cmd.entity_id);
                            validated_attacks.push((cmd.entity_id, target_id));
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Succeeded);
                        }
                    }
                }
                CommandPayload::Purchase { item_id, price } => {
                    if let Some(bevy_entity) = self.world.get_entity(cmd.entity_id) {
                        let mut entity_mut = self.world.world.entity_mut(bevy_entity);
                        let mut has_sufficient_gold = false;
                        if let Some(mut inv) = entity_mut.get_mut::<InventoryProjection>() {
                            for slot in &mut inv.slots {
                                if (slot.item_id.contains("gold") || slot.item_id.contains("coin"))
                                    && price >= 0
                                    && i64::from(slot.quantity) >= price
                                {
                                    slot.quantity -= price as u32;
                                    has_sufficient_gold = true;
                                    break;
                                }
                            }
                            if has_sufficient_gold {
                                let item_name = if item_id == 1001 {
                                    "food".to_string()
                                } else {
                                    format!("item_{item_id}")
                                };
                                if let Some(item_slot) =
                                    inv.slots.iter_mut().find(|s| s.item_id == item_name)
                                {
                                    item_slot.quantity += 1;
                                } else {
                                    let slot_index = inv.slots.len() as u16;
                                    inv.slots.push(game_domain::state::InventorySlotDto {
                                        slot_index,
                                        item_id: item_name,
                                        quantity: 1,
                                    });
                                }
                            }
                        } else {
                            // Entities without inventory projection (e.g. contracts/unit tests) are permitted
                            has_sufficient_gold = true;
                        }

                        if has_sufficient_gold {
                            emit(
                                SimulationEvent::PurchaseExecuted {
                                    buyer_id: cmd.entity_id,
                                    item_id,
                                    price,
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Succeeded);
                        } else {
                            emit(
                                SimulationEvent::CommandRejected {
                                    command_id: cmd.command_id,
                                    entity_id: cmd.entity_id,
                                    reason: "insufficient gold or missing inventory".to_string(),
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                            let _ = self
                                .deduplicator
                                .update_status(cmd.command_id, CommandStatus::Failed);
                        }
                    } else {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: "buyer entity not found".to_string(),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                    }
                }
                CommandPayload::Custom { action, payload } => {
                    let mut succeeded = false;
                    let mut rejection_reason = "unsupported custom action".to_string();

                    if let Some(bevy_entity) = self.world.get_entity(cmd.entity_id) {
                        let mut entity_mut = self.world.world.entity_mut(bevy_entity);
                        match action.as_str() {
                            "npc_eat_food" => {
                                let mut has_food = false;
                                if let Some(mut inv) = entity_mut.get_mut::<InventoryProjection>() {
                                    for slot in &mut inv.slots {
                                        let lower = slot.item_id.to_ascii_lowercase();
                                        if (lower.contains("food")
                                            || lower.contains("bread")
                                            || lower.contains("apple")
                                            || lower.contains("ration"))
                                            && slot.quantity > 0
                                        {
                                            slot.quantity -= 1;
                                            has_food = true;
                                            break;
                                        }
                                    }
                                }

                                if !has_food {
                                    rejection_reason =
                                        "missing edible food item in inventory".to_string();
                                } else if let Some(mut needs) = entity_mut.get_mut::<Needs>() {
                                    needs.hunger = needs.hunger.saturating_sub(400);
                                    succeeded = true;
                                } else {
                                    rejection_reason = "entity lacks Needs component".to_string();
                                }
                            }
                            "npc_rest" => {
                                if let Some(mut needs) = entity_mut.get_mut::<Needs>() {
                                    needs.fatigue = needs.fatigue.saturating_sub(500);
                                    succeeded = true;
                                } else {
                                    rejection_reason = "entity lacks Needs component".to_string();
                                }
                            }
                            "npc_work" => {
                                if let Some(mut needs) = entity_mut.get_mut::<Needs>() {
                                    needs.fatigue = needs.fatigue.saturating_add(50);
                                    succeeded = true;
                                } else {
                                    rejection_reason = "entity lacks Needs component".to_string();
                                }
                            }
                            _ => {
                                // Generic custom actions for rule engine / external triggers
                                succeeded = true;
                            }
                        }
                    } else {
                        rejection_reason = "target entity not found".to_string();
                    }

                    if succeeded {
                        emit(
                            SimulationEvent::Custom {
                                name: action,
                                data: format!("bytes:{}", payload.len()),
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Succeeded);
                    } else {
                        emit(
                            SimulationEvent::CommandRejected {
                                command_id: cmd.command_id,
                                entity_id: cmd.entity_id,
                                reason: rejection_reason,
                            },
                            &mut self.event_budget,
                            &mut events,
                        );
                        let _ = self
                            .deduplicator
                            .update_status(cmd.command_id, CommandStatus::Failed);
                    }
                }
                CommandPayload::Noop => {
                    let _ = self
                        .deduplicator
                        .update_status(cmd.command_id, CommandStatus::Succeeded);
                }
            }
        }

        // 2. Perception Phase: Spatial indexing updates
        let _ = TickPhase::Perception;

        // 3. NavigationAndMovement Phase: Apply validated moves and velocities
        for (entity_id, target_x, target_y) in validated_moves {
            if let Some(entity) = self.world.get_entity(entity_id) {
                let mut entity_mut = self.world.world.entity_mut(entity);
                if let Some(mut nav) = entity_mut.get_mut::<Navigation>() {
                    nav.target_x_millitile = Some(target_x);
                    nav.target_y_millitile = Some(target_y);
                    nav.state = NavigationState::PathRequested;
                    nav.path_version = nav.path_version.wrapping_add(1);
                    nav.failure_reason = None;
                    nav.waypoints.clear();
                    nav.current_waypoint_idx = 0;
                } else {
                    let pos = entity_mut
                        .get::<Position>()
                        .copied()
                        .unwrap_or_else(|| Position::new(0, 0));
                    let dx = ((target_x as i128 - pos.x_millitile as i128).clamp(-500, 500)) as i32;
                    let dy = ((target_y as i128 - pos.y_millitile as i128).clamp(-500, 500)) as i32;
                    if let Some(mut vel) = entity_mut.get_mut::<Velocity>() {
                        vel.dx = dx;
                        vel.dy = dy;
                    }
                }
            }
        }

        // Iterate through entities in deterministic sorted EntityId order
        let mut sorted_entity_ids: Vec<EntityId> =
            self.world.index.iter().map(|(id, _)| *id).collect();
        sorted_entity_ids.sort();

        for id in &sorted_entity_ids {
            if let Some(entity) = self.world.get_entity(*id) {
                let mut entity_mut = self.world.world.entity_mut(entity);
                if let (Some(mut nav), Some(mut pos)) = (
                    entity_mut.get::<Navigation>().cloned(),
                    entity_mut.get::<Position>().copied(),
                ) {
                    if let (NavigationState::PathRequested, Some(gx), Some(gy)) =
                        (nav.state, nav.target_x_millitile, nav.target_y_millitile)
                    {
                        match crate::pathfinding::astar_path(
                            &self.world.grid,
                            (pos.x_millitile, pos.y_millitile),
                            (gx, gy),
                            512,
                        ) {
                            Ok(waypoints) => {
                                nav.waypoints = waypoints;
                                nav.current_waypoint_idx = 0;
                                nav.state = NavigationState::Following;
                                nav.failure_reason = None;
                                emit(
                                    SimulationEvent::NavigationStateChanged {
                                        entity_id: *id,
                                        state: NavigationState::Following,
                                        reason: None,
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                            }
                            Err(err) => {
                                let reason_str = err.to_string();
                                nav.state = NavigationState::Blocked;
                                nav.failure_reason = Some(reason_str.clone());
                                emit(
                                    SimulationEvent::NavigationStateChanged {
                                        entity_id: *id,
                                        state: NavigationState::Blocked,
                                        reason: Some(reason_str),
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                            }
                        }
                    }

                    if nav.state == NavigationState::Following {
                        if nav.current_waypoint_idx >= nav.waypoints.len() {
                            nav.state = NavigationState::Arrived;
                            emit(
                                SimulationEvent::NavigationStateChanged {
                                    entity_id: *id,
                                    state: NavigationState::Arrived,
                                    reason: None,
                                },
                                &mut self.event_budget,
                                &mut events,
                            );
                        } else {
                            let next_wp = nav.waypoints[nav.current_waypoint_idx];
                            let obstacle_detected =
                                !self.world.grid.is_millitile_walkable(next_wp.0, next_wp.1);
                            if obstacle_detected {
                                match (nav.target_x_millitile, nav.target_y_millitile) {
                                    (Some(gx), Some(gy)) => {
                                        match crate::pathfinding::astar_path(
                                            &self.world.grid,
                                            (pos.x_millitile, pos.y_millitile),
                                            (gx, gy),
                                            512,
                                        ) {
                                            Ok(new_waypoints) => {
                                                nav.waypoints = new_waypoints;
                                                nav.current_waypoint_idx = 0;
                                            }
                                            Err(err) => {
                                                let reason = format!("path obstructed: {err}");
                                                nav.state = NavigationState::Blocked;
                                                nav.failure_reason = Some(reason.clone());
                                                emit(
                                                    SimulationEvent::NavigationStateChanged {
                                                        entity_id: *id,
                                                        state: NavigationState::Blocked,
                                                        reason: Some(reason),
                                                    },
                                                    &mut self.event_budget,
                                                    &mut events,
                                                );
                                            }
                                        }
                                    }
                                    _ => {
                                        let reason = "path obstructed: no target".to_string();
                                        nav.state = NavigationState::Blocked;
                                        nav.failure_reason = Some(reason.clone());
                                        emit(
                                            SimulationEvent::NavigationStateChanged {
                                                entity_id: *id,
                                                state: NavigationState::Blocked,
                                                reason: Some(reason),
                                            },
                                            &mut self.event_budget,
                                            &mut events,
                                        );
                                    }
                                }
                            }

                            if nav.state == NavigationState::Following
                                && nav.current_waypoint_idx < nav.waypoints.len()
                            {
                                let wp = nav.waypoints[nav.current_waypoint_idx];
                                let dx_128 = wp.0 as i128 - pos.x_millitile as i128;
                                let dy_128 = wp.1 as i128 - pos.y_millitile as i128;
                                let dist_sq = (dx_128.unsigned_abs() * dx_128.unsigned_abs())
                                    .saturating_add(dy_128.unsigned_abs() * dy_128.unsigned_abs());
                                let dist = ((dist_sq as f64).sqrt()) as i64;
                                let dist = dist.max(1);
                                let speed = nav.speed_millitiles_per_tick.max(50);

                                let from_x = pos.x_millitile;
                                let from_y = pos.y_millitile;

                                if dist <= speed {
                                    pos.x_millitile = wp.0;
                                    pos.y_millitile = wp.1;
                                    nav.current_waypoint_idx += 1;
                                    if nav.current_waypoint_idx >= nav.waypoints.len() {
                                        nav.state = NavigationState::Arrived;
                                        emit(
                                            SimulationEvent::NavigationStateChanged {
                                                entity_id: *id,
                                                state: NavigationState::Arrived,
                                                reason: None,
                                            },
                                            &mut self.event_budget,
                                            &mut events,
                                        );
                                    }
                                } else {
                                    let step_x = ((dx_128 * speed as i128) / dist as i128) as i64;
                                    let step_y = ((dy_128 * speed as i128) / dist as i128) as i64;
                                    pos.x_millitile = pos.x_millitile.saturating_add(step_x);
                                    pos.y_millitile = pos.y_millitile.saturating_add(step_y);
                                }

                                emit(
                                    SimulationEvent::EntityMoved {
                                        entity_id: *id,
                                        from_x,
                                        from_y,
                                        to_x: pos.x_millitile,
                                        to_y: pos.y_millitile,
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                            }
                        }
                    }

                    if let Some(mut n) = entity_mut.get_mut::<Navigation>() {
                        *n = nav;
                    }
                    if let Some(mut p) = entity_mut.get_mut::<Position>() {
                        *p = pos;
                    }
                } else {
                    let vel_opt = entity_mut.get::<Velocity>().copied();
                    match (vel_opt, entity_mut.get_mut::<Position>()) {
                        (Some(vel), Some(mut pos)) if vel.dx != 0 || vel.dy != 0 => {
                            const MAX_FALLBACK_SPEED: i64 = 500;
                            let dx = (vel.dx as i64).clamp(-MAX_FALLBACK_SPEED, MAX_FALLBACK_SPEED);
                            let dy = (vel.dy as i64).clamp(-MAX_FALLBACK_SPEED, MAX_FALLBACK_SPEED);
                            let new_x = pos.x_millitile.saturating_add(dx);
                            let new_y = pos.y_millitile.saturating_add(dy);

                            if self.world.grid.is_in_bounds_millitile(new_x, new_y)
                                && self.world.grid.is_millitile_walkable(new_x, new_y)
                            {
                                let from_x = pos.x_millitile;
                                let from_y = pos.y_millitile;
                                pos.x_millitile = new_x;
                                pos.y_millitile = new_y;
                                emit(
                                    SimulationEvent::EntityMoved {
                                        entity_id: *id,
                                        from_x,
                                        from_y,
                                        to_x: new_x,
                                        to_y: new_y,
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                            } else if let Some(mut v) = entity_mut.get_mut::<Velocity>() {
                                v.dx = 0;
                                v.dy = 0;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // 4. Combat Phase: Resolve combat interactions
        for (attacker_id, target_id) in validated_attacks {
            let attacker_stats_opt =
                if let Some(attacker_entity) = self.world.get_entity(attacker_id) {
                    let mut attacker_mut = self.world.world.entity_mut(attacker_entity);
                    if let Some(mut stats) = attacker_mut.get_mut::<CombatStats>() {
                        stats.last_attack_tick = current_tick;
                        Some(*stats)
                    } else {
                        None
                    }
                } else {
                    None
                };

            if let Some(target_entity) = self.world.get_entity(target_id) {
                let mut target_mut = self.world.world.entity_mut(target_entity);
                let target_armor = target_mut
                    .get::<CombatStats>()
                    .map(|s| s.armor)
                    .unwrap_or(0);
                let archetype_opt = target_mut.get::<MonsterArchetype>().copied();

                if let Some(mut health) = target_mut
                    .get_mut::<Health>()
                    .filter(|h| h.status == CombatStatus::Alive)
                {
                    let base_dmg = attacker_stats_opt
                        .map(|s| s.base_damage)
                        .unwrap_or_else(|| self.rng.gen_range_inclusive(8, 12));
                    let damage = (base_dmg - target_armor).max(1);

                    health.current = (health.current - damage).max(0);
                    let remaining_hp = health.current;
                    emit(
                        SimulationEvent::CombatHit {
                            attacker_id,
                            target_id,
                            damage,
                            remaining_hp,
                        },
                        &mut self.event_budget,
                        &mut events,
                    );
                    if health.current == 0 {
                        health.status = CombatStatus::Dead;
                        emit(
                            SimulationEvent::EntityDied {
                                entity_id: target_id,
                            },
                            &mut self.event_budget,
                            &mut events,
                        );

                        match archetype_opt {
                            Some(archetype) if self.produced_loot_victims.insert(target_id) => {
                                let (item_id, amount) = archetype.default_loot();
                                let seed_string = format!(
                                    "loot:{}:{}:{}:{}",
                                    attacker_id.0, target_id.0, item_id, amount
                                );
                                let hash = Sha256::digest(seed_string.as_bytes());
                                let mut uuid_bytes = [0u8; 16];
                                uuid_bytes.copy_from_slice(&hash[..16]);
                                uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
                                uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
                                let intent_id = Uuid::from_bytes(uuid_bytes);
                                emit(
                                    SimulationEvent::LootIntentProduced {
                                        killer_id: attacker_id,
                                        victim_id: target_id,
                                        item_id,
                                        amount,
                                        intent_id,
                                    },
                                    &mut self.event_budget,
                                    &mut events,
                                );
                                self.pending_loot_intents.push(DurableLootIntent {
                                    intent_id,
                                    killer_id: attacker_id,
                                    victim_id: target_id,
                                    item_id,
                                    amount,
                                    tick: current_tick,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // 5. NpcDecisions Phase: Scheduled tactical planning
        let mut executed_npc_decisions = 0;
        if self.npc_decision_interval > 0 && current_tick.is_multiple_of(self.npc_decision_interval)
        {
            for id in &sorted_entity_ids {
                if let Some(entity) = self.world.get_entity(*id) {
                    let mut entity_mut = self.world.world.entity_mut(entity);
                    if entity_mut.get::<Personality>().is_some() {
                        let is_alive = entity_mut
                            .get::<Health>()
                            .is_none_or(|h| h.status == CombatStatus::Alive);
                        if is_alive {
                            *self.npc_decision_counts.entry(*id).or_insert(0) += 1;
                            executed_npc_decisions += 1;
                            emit(
                                SimulationEvent::NpcDecisionExecuted {
                                    entity_id: *id,
                                    tick: current_tick,
                                },
                                &mut self.event_budget,
                                &mut events,
                            );

                            // Deterministic wander decision
                            let dx = self.rng.gen_range_inclusive(-100, 100);
                            let dy = self.rng.gen_range_inclusive(-100, 100);
                            if let Some(mut vel) = entity_mut.get_mut::<Velocity>() {
                                vel.dx = dx;
                                vel.dy = dy;
                            }
                        }
                    }
                }
            }
        }

        // 6. WorldEconomy Phase: (Needs decay / market tick placeholder)
        let _ = TickPhase::WorldEconomy;

        // 7. EventRules Phase: Trigger rules evaluation
        let _ = TickPhase::EventRules;
        let mut event_envelopes: Vec<EventEnvelope> = events
            .iter()
            .enumerate()
            .map(|(seq, ev)| {
                EventEnvelope::new_root(
                    self.world.meta.region_id,
                    current_tick,
                    seq as u64,
                    None,
                    None,
                    ev.clone(),
                )
            })
            .collect();

        let initial_envelopes = event_envelopes.clone();
        for root_envelope in &initial_envelopes {
            let (child_envelopes, follow_up_commands) =
                self.rule_engine
                    .process_event(root_envelope, current_tick, &mut self.event_budget);

            for child in child_envelopes {
                if self.event_budget.max_events_per_tick > 0
                    && events.len() < self.event_budget.max_events_per_tick
                {
                    events.push(child.payload.clone());
                    event_envelopes.push(child);
                }
            }

            for cmd in follow_up_commands {
                self.command_queue.enqueue(cmd.to_ingress());
            }
        }

        // 8. Finalize Phase: Package tick completion event
        let tick_event = SimulationEvent::TickCompleted {
            tick: current_tick,
            entity_count: self.world.entity_count(),
        };
        let before_len = events.len();
        emit(tick_event, &mut self.event_budget, &mut events);
        if events.len() > before_len {
            let seq = event_envelopes.len() as u64;
            event_envelopes.push(EventEnvelope::new_root(
                self.world.meta.region_id,
                current_tick,
                seq,
                None,
                None,
                events.last().unwrap().clone(),
            ));
        }

        // 9. Output Phase: Calculate canonical state hash and package output
        let state_hash = self.compute_state_hash(current_tick, &events);
        let durable_loot_intents = self.pending_loot_intents.clone();

        Ok(TickOutput {
            tick: current_tick,
            state_hash,
            events,
            event_envelopes,
            clock_status: self.clock.status,
            executed_npc_decisions,
            durable_loot_intents,
        })
    }

    /// Computes a deterministic SHA-256 state hash for the current world snapshot and tick events.
    pub fn compute_state_hash(&mut self, tick: u64, events: &[SimulationEvent]) -> String {
        let snapshot = self.world.export_canonical();
        let mut hasher = Sha256::new();
        hasher.update(tick.to_le_bytes());

        let snapshot_json =
            serde_json::to_vec(&snapshot).expect("deterministic snapshot serialization");
        hasher.update(&snapshot_json);

        let events_json = serde_json::to_vec(events).expect("deterministic events serialization");
        hasher.update(&events_json);

        let result = hasher.finalize();
        format!("{result:x}")
    }

    /// Executes multiple headless ticks synchronously without wall-clock sleep.
    ///
    /// # Errors
    /// Returns `DomainError` if any step returns an error.
    pub fn run_ticks(&mut self, total_ticks: u64) -> Result<SimulationSummary, DomainError> {
        let start_tick = self.clock.current_tick + 1;
        let mut total_events = 0;
        let mut last_hash = String::new();

        for _ in 0..total_ticks {
            let output = self.step()?;
            total_events += output.events.len();
            last_hash = output.state_hash;
        }

        let npc_counts = self
            .npc_decision_counts
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();

        Ok(SimulationSummary {
            seed: self.rng.seed(),
            start_tick,
            end_tick: self.clock.current_tick,
            total_ticks_executed: total_ticks,
            final_state_hash: last_hash,
            total_events_emitted: total_events,
            clock_status: self.clock.status,
            npc_decision_counts: npc_counts,
        })
    }
}
