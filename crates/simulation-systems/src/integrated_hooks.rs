//! Simulation engine hooks implementation integrating all domain systems.
//!
//! Provides `IntegratedSimulationHooks`, which wires:
//! - Phase 2 & 5: NPC tactical decision-making (`NpcAiSystem`, `NpcScheduler`, `NpcAgentState`)
//! - Phase 6: Biological needs decay (`update_needs`), memory decay, and settlement economy (`SettlementEconomy`, `FoodLedger`)
//! - Phase 7: Dynamic event chains (`CaravanEventChain`) and DSL-driven dynamic rules (`DynamicRuleRegistry`)

use crate::economy::{FoodLedger, SettlementEconomy};
use crate::event_chain::CaravanEventChain;
use crate::faction::FactionManager;
use crate::memory::MemoryManager;
use crate::needs::update_needs;
use crate::rule_dsl::{ActionAST, DynamicRuleRegistry, MetricContext, MetricKey};
use crate::scheduler::NpcScheduler;
use crate::system::{NpcAgentState, NpcAiSystem};

use ecs_components::components::{MemoryRef, Needs, Personality};
use ecs_components::region_world::RegionWorld;
use game_domain::id::{CommandId, EntityId, RegionId, SessionId};
use simulation_core::command::{CommandEnvelope, CommandStatus};
use simulation_core::event::SimulationEvent;
use simulation_core::hook::SimulationPhaseHooks;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Integrated simulation phase hooks combining AI, Needs, Memory, Economy, and Dynamic Events.
#[derive(Debug)]
pub struct IntegratedSimulationHooks {
    pub ai_system: NpcAiSystem,
    pub scheduler: NpcScheduler,
    pub agents: BTreeMap<EntityId, NpcAgentState>,
    pub memory_manager: MemoryManager,
    pub economy: SettlementEconomy,
    pub food_ledger: FoodLedger,
    pub caravan_chain: Option<CaravanEventChain>,
    pub faction_manager: FactionManager,
    pub dynamic_rules: DynamicRuleRegistry,
    pub prev_metric_ctx: MetricContext,
    pub session_id: SessionId,
    pub region_id: RegionId,
    pub weather_scale: i64,
}

impl Default for IntegratedSimulationHooks {
    fn default() -> Self {
        Self {
            ai_system: NpcAiSystem::default(),
            scheduler: NpcScheduler::default(),
            agents: BTreeMap::new(),
            memory_manager: MemoryManager,
            economy: SettlementEconomy::new_default(1),
            food_ledger: FoodLedger::default(),
            caravan_chain: None,
            faction_manager: FactionManager::default(),
            dynamic_rules: DynamicRuleRegistry::default(),
            prev_metric_ctx: MetricContext::default(),
            session_id: SessionId(Uuid::nil()),
            region_id: RegionId(Uuid::nil()),
            weather_scale: 100,
        }
    }
}

impl IntegratedSimulationHooks {
    /// Creates a new integrated hooks bundle.
    #[must_use]
    pub fn new(
        ai_system: NpcAiSystem,
        scheduler: NpcScheduler,
        economy: SettlementEconomy,
        faction_manager: FactionManager,
        dynamic_rules: DynamicRuleRegistry,
        session_id: SessionId,
        region_id: RegionId,
    ) -> Self {
        Self {
            ai_system,
            scheduler,
            agents: BTreeMap::new(),
            memory_manager: MemoryManager,
            economy,
            food_ledger: FoodLedger::default(),
            caravan_chain: None,
            faction_manager,
            dynamic_rules,
            prev_metric_ctx: MetricContext::default(),
            session_id,
            region_id,
            weather_scale: 100,
        }
    }
}

impl SimulationPhaseHooks for IntegratedSimulationHooks {
    fn on_npc_decisions(
        &mut self,
        world: &mut RegionWorld,
        current_tick: u64,
        executed_decisions: &mut usize,
        events: &mut Vec<SimulationEvent>,
        commands: &mut Vec<CommandEnvelope>,
        command_outcomes: &BTreeMap<CommandId, CommandStatus>,
    ) {
        let initial_cmds_count = commands.len();
        self.ai_system.step_simulation_world(
            world,
            &self.scheduler,
            &mut self.agents,
            current_tick,
            self.session_id,
            events,
            commands,
            command_outcomes,
        );
        *executed_decisions += commands.len().saturating_sub(initial_cmds_count);
    }

    fn on_world_economy(
        &mut self,
        world: &mut RegionWorld,
        current_tick: u64,
        events: &mut Vec<SimulationEvent>,
    ) {
        // 1. Biological needs decay for all living NPCs with Needs & memory pruning
        let mut sorted_entity_ids: Vec<EntityId> =
            world.index.iter().map(|(id, _)| *id).collect();
        sorted_entity_ids.sort();

        for id in &sorted_entity_ids {
            if let Some(entity) = world.get_entity(*id) {
                let mut entity_mut = world.world.entity_mut(entity);
                if entity_mut.get::<Personality>().is_some() {
                    if let Some(mut needs) = entity_mut.get_mut::<Needs>() {
                        // In routine economy phase, decay needs according to resting/working status
                        let is_resting = needs.fatigue > 700;
                        let default_facts = crate::perception::NpcWorldFacts::default();
                        update_needs(&mut needs, &default_facts, is_resting, false);
                    }
                    if let Some(mut memory_ref) = entity_mut.get_mut::<MemoryRef>() {
                        MemoryManager::prune_expired(&mut memory_ref, current_tick);
                    }
                }
            }
        }

        // 2. Settlement economy simulation cycle
        let eco_events = self.economy.step_economy(
            current_tick,
            self.weather_scale,
            &mut self.food_ledger,
        );
        for ev in eco_events {
            events.push(ev);
        }
    }

    fn on_event_rules(
        &mut self,
        _world: &mut RegionWorld,
        current_tick: u64,
        events: &mut Vec<SimulationEvent>,
        _commands: &mut Vec<CommandEnvelope>,
    ) {
        let sid = self.economy.settlement_id;

        // 1. Update metric context for dynamic rules
        let mut curr_metric_ctx = MetricContext::new();
        curr_metric_ctx.set(MetricKey::FoodStock(sid), self.economy.food_stock);
        curr_metric_ctx.set(MetricKey::MarketPrice(sid), self.economy.current_price);
        curr_metric_ctx.set(MetricKey::SecurityIndex(sid), self.economy.security_index as i64);
        curr_metric_ctx.set(
            MetricKey::ShortageActive(sid),
            if self.economy.shortage_active { 1 } else { 0 },
        );

        // 2. Evaluate dynamic rules DSL
        let triggered_actions = self.dynamic_rules.evaluate(
            current_tick,
            &self.prev_metric_ctx,
            &curr_metric_ctx,
        );

        for action in triggered_actions {
            match action {
                ActionAST::EmitEvent(ev) => {
                    events.push(ev);
                }
                ActionAST::EmitNews { headline } => {
                    events.push(SimulationEvent::Custom {
                        name: "NewsBroadcast".to_string(),
                        data: headline,
                    });
                }
                ActionAST::DispatchCaravan {
                    caravan_id,
                    settlement_id,
                    food_amount,
                } => {
                    if self.caravan_chain.is_none() {
                        let chain = CaravanEventChain::new_shortage_request(
                            caravan_id,
                            settlement_id,
                            current_tick,
                            food_amount,
                        );
                        self.caravan_chain = Some(chain);
                    }
                }
                ActionAST::AdjustSecurity { settlement_id: _, delta } => {
                    self.economy.security_index =
                        (self.economy.security_index + delta).clamp(0, 100);
                }
            }
        }

        self.prev_metric_ctx = curr_metric_ctx;

        // 3. Step active CaravanEventChain if present
        if let Some(ref mut chain) = self.caravan_chain {
            let chain_events = chain.step(
                current_tick,
                &mut self.economy,
                &mut self.faction_manager,
                &mut self.food_ledger,
                None, // No direct player actor override in headless cycle
                None, // Autonomous caravan journey
            );
            for ev in chain_events {
                events.push(ev);
            }
        }

        // Clean up resolved caravan chain so future caravans can spawn
        if self.caravan_chain.as_ref().is_some_and(|c| !c.is_active()) {
            self.caravan_chain = None;
        }
    }
}
