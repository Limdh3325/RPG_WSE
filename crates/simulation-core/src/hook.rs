use crate::command::{CommandEnvelope, CommandStatus};
use crate::event::SimulationEvent;
use ecs_components::region_world::RegionWorld;
use game_domain::CommandId;
use std::collections::BTreeMap;

/// Hook interface executed during specific phases of the `SimulationEngine` tick pipeline.
///
/// Implementors can inject AI tactical planning, biological needs updates,
/// regional economy simulations, and dynamic event chains without tight coupling
/// or circular dependencies.
pub trait SimulationPhaseHooks: Send + Sync {
    /// Phase 2 / Phase 5: Executed during the NPC decision phase.
    ///
    /// The hook can inspect/mutate entities in `world`, generate autonomous commands
    /// to be enqueued for subsequent ticks, and emit simulation events.
    fn on_npc_decisions(
        &mut self,
        _world: &mut RegionWorld,
        _current_tick: u64,
        _executed_decisions: &mut usize,
        _events: &mut Vec<SimulationEvent>,
        _commands: &mut Vec<CommandEnvelope>,
        _command_outcomes: &BTreeMap<CommandId, CommandStatus>,
    ) {
    }

    /// Phase 6: Executed during the WorldEconomy phase.
    ///
    /// Updates entity needs (hunger, fatigue) and settlement economy (food, markets).
    fn on_world_economy(
        &mut self,
        _world: &mut RegionWorld,
        _current_tick: u64,
        _events: &mut Vec<SimulationEvent>,
    ) {
    }

    /// Phase 7: Executed during the EventRules phase before rule evaluation.
    ///
    /// Steps event chains (e.g. caravans, invasions) and dynamic rule triggers.
    fn on_event_rules(
        &mut self,
        _world: &mut RegionWorld,
        _current_tick: u64,
        _events: &mut Vec<SimulationEvent>,
        _commands: &mut Vec<CommandEnvelope>,
    ) {
    }
}

/// Default no-op hooks implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSimulationHooks;

impl SimulationPhaseHooks for NoopSimulationHooks {}
