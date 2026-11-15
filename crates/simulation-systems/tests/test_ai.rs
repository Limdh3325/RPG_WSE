//! Verification tests for TASK-010: NPC Utility AI / GOAP Engine.
//!
//! Covers:
//! - TEST-AI-001: Threat evasion / FLEE priority over EAT under threat
//! - TEST-AI-002: Plan synthesis for BUY -> EAT when out of food with gold & open shop
//! - TEST-AI-003: Re-planning on invalid route and fallback to Idle on 3 consecutive failures
//! - TEST-AI-004: Deterministic goal and plan reproducibility given identical inputs
//! - TEST-AI-005: LLM boundary protection against arbitrary/non-existent item grants
//! - TEST-AI-006: Memory deduplication preventing multiple entries with identical source_event_id
//! - DOD-NPC-10: 10 independent NPCs with routine behaviors and threat response
//! - DOD-NPC-1000-BUDGET: 1,000 NPCs evaluated under tick budget constraints

use ecs_components::{CombatStats, MemoryRef, MonsterArchetype, Needs, Personality, Position};
use game_domain::EntityId;
use simulation_core::event::SimulationEvent;
use simulation_systems::goap::{GoapActionId, GoapGoal, GoapPlanner, GoapPlannerConfig, GoapState};
use simulation_systems::llm_boundary::{
    AuthoritativeItemRegistry, LlmItemGrantProposal, validate_and_grant_item,
};
use simulation_systems::memory::MemoryManager;
use simulation_systems::perception::{NpcWorldFacts, PerceptionContext, SurroundingEntityData};
use simulation_systems::scheduler::{NpcScheduler, NpcSchedulerConfig};
use simulation_systems::system::{NpcAgentState, NpcAiLifecycleState, NpcAiSystem};
use simulation_systems::utility::{NpcGoal, UtilityEvaluator};
use uuid::Uuid;

#[test]
fn test_ai_001_flee_takes_precedence_over_eat_under_threat() {
    let evaluator = UtilityEvaluator::default();

    // High hunger (850/1000)
    let needs = Needs {
        hunger: 850,
        fatigue: 200,
        safety: 300,
    };
    let personality = Personality {
        courage: 0,
        greed: 100,
        loyalty: 0,
    };

    // Imminent threat present (level 700)
    let facts = NpcWorldFacts {
        threat_present: true,
        threat_level: 700,
        threat_entity_id: Some(EntityId::new()),
        threat_pos: None,
        is_safe: false,
        in_combat: false,
        has_food: true,
        has_gold: true,
        shop_open: false,
        at_market: false,
        near_home: false,
        hunger_reduced: false,
        fatigue_reduced: false,
        work_done: false,
    };

    let flee_score = evaluator.evaluate_score(NpcGoal::Flee, &facts, &needs, &personality);
    let eat_score = evaluator.evaluate_score(NpcGoal::Eat, &facts, &needs, &personality);

    assert!(
        flee_score > eat_score,
        "FLEE score ({flee_score}) must strictly exceed EAT score ({eat_score}) when threat is imminent (TEST-AI-001)"
    );

    let (chosen_goal, score) =
        evaluator.select_goal(None, &facts, &needs, &personality, EntityId::new());

    assert_eq!(
        chosen_goal,
        NpcGoal::Flee,
        "Chosen goal must be FLEE when threat is present despite severe hunger"
    );
    assert_eq!(score, flee_score);
}

#[test]
fn test_ai_002_buy_to_eat_plan_generated_when_no_food_but_has_gold() {
    let planner = GoapPlanner::new(GoapPlannerConfig::default());

    // State: No food, has gold, shop open, not at market, safe
    let initial_state = GoapState {
        threat_present: false,
        is_safe: true,
        in_combat: false,
        has_food: false,
        hunger_reduced: false,
        fatigue_reduced: false,
        near_home: false,
        has_gold: true,
        shop_open: true,
        at_market: false,
        work_done: false,
    };

    let plan = planner
        .plan(initial_state, GoapGoal::Eat)
        .expect("Planner must synthesize a valid action sequence");

    assert_eq!(
        plan,
        vec![
            GoapActionId::MoveToMarket,
            GoapActionId::BuyFood,
            GoapActionId::EatFood,
        ],
        "Plan must chain MOVE_TO_MARKET -> BUY_FOOD -> EAT_FOOD (TEST-AI-002)"
    );
}

#[test]
fn test_ai_003_replan_and_fallback_to_idle_on_consecutive_failures() {
    let ai_system = NpcAiSystem::default();
    let entity_id = EntityId::new();
    let mut agent = NpcAgentState::new(entity_id);

    let needs = Needs {
        hunger: 800,
        fatigue: 100,
        safety: 1000,
    };
    let personality = Personality::default();

    let ctx = PerceptionContext {
        self_entity_id: entity_id,
        self_pos: Position {
            x_millitile: 0,
            y_millitile: 0,
        },
        self_faction: None,
        inventory: None,
        market_pos: None,
        home_pos: None,
        region_id: game_domain::RegionId::new(),
        session_id: game_domain::SessionId::from_u128(0),
    };

    let surrounding: &[SurroundingEntityData] = &[];

    let mut events = Vec::new();

    // Tick 1: Action precondition fails when executing BuyFood without being at market
    agent.current_plan = vec![GoapActionId::BuyFood];
    agent.step_index = 0;
    agent.lifecycle = NpcAiLifecycleState::Executing;
    ai_system.step_npc(
        &mut agent,
        &ctx,
        surrounding,
        &needs,
        &personality,
        1,
        &mut events,
    );
    assert_eq!(agent.consecutive_failures, 1);

    // Failure 2 (Tick 2)
    agent.current_plan = vec![GoapActionId::BuyFood];
    agent.step_index = 0;
    agent.lifecycle = NpcAiLifecycleState::Executing;
    ai_system.step_npc(
        &mut agent,
        &ctx,
        surrounding,
        &needs,
        &personality,
        2,
        &mut events,
    );
    assert_eq!(agent.consecutive_failures, 2);

    // Failure 3 (Tick 3) -> Triggers fallback to Idle with cooldown
    agent.current_plan = vec![GoapActionId::BuyFood];
    agent.step_index = 0;
    agent.lifecycle = NpcAiLifecycleState::Executing;
    ai_system.step_npc(
        &mut agent,
        &ctx,
        surrounding,
        &needs,
        &personality,
        3,
        &mut events,
    );
    assert_eq!(agent.consecutive_failures, 3);
    assert_eq!(
        agent.lifecycle,
        NpcAiLifecycleState::Idle,
        "Agent must fall back to Idle on 3 consecutive failures (TEST-AI-003)"
    );
    assert!(
        agent.cooldown_until_tick > 3,
        "Cooldown must be scheduled after 3 failures"
    );

    // Verify diagnostic event emitted
    let has_plan_failed_event = events.iter().any(|e| {
        matches!(e, SimulationEvent::NpcPlanFailed { consecutive_failures, .. } if *consecutive_failures >= 3)
    });
    assert!(
        has_plan_failed_event,
        "NpcPlanFailed diagnostic event must be emitted on fallback"
    );
}

#[test]
fn test_ai_004_determinism_identical_inputs_produce_identical_outputs() {
    let evaluator = UtilityEvaluator::default();
    let planner = GoapPlanner::default();

    let needs = Needs {
        hunger: 650,
        fatigue: 420,
        safety: 800,
    };
    let personality = Personality {
        courage: 50,
        greed: 150,
        loyalty: 80,
    };
    let facts = NpcWorldFacts {
        threat_present: false,
        threat_level: 0,
        threat_entity_id: None,
        threat_pos: None,
        is_safe: true,
        in_combat: false,
        has_food: false,
        has_gold: true,
        shop_open: true,
        at_market: false,
        near_home: false,
        hunger_reduced: false,
        fatigue_reduced: false,
        work_done: false,
    };
    let entity_id = EntityId::new();

    let (baseline_goal, baseline_score) =
        evaluator.select_goal(None, &facts, &needs, &personality, entity_id);
    let baseline_plan = planner.plan((&facts).into(), baseline_goal.into());

    for _ in 0..100 {
        let (goal, score) = evaluator.select_goal(None, &facts, &needs, &personality, entity_id);
        let plan = planner.plan((&facts).into(), goal.into());

        assert_eq!(goal, baseline_goal);
        assert_eq!(score, baseline_score);
        assert_eq!(plan, baseline_plan);
    }
}

#[test]
fn test_ai_005_llm_boundary_rejects_unregistered_items_and_grants_zero() {
    let registry = AuthoritativeItemRegistry::default();
    let target = EntityId::new();

    // 1. Proposal with completely bogus/hallucinated item ID
    let bogus_proposal = LlmItemGrantProposal {
        target_entity_id: target,
        item_id: "item_infinite_god_sword_9999".to_string(),
        quantity: 1,
        source: "llm_narrative_reward".to_string(),
    };

    let result = validate_and_grant_item(&registry, &bogus_proposal);
    assert!(
        result.is_err(),
        "Server verification must reject unregistered item proposals (TEST-AI-005)"
    );

    // 2. Proposal exceeding maximum allowed grant quantity
    let excess_proposal = LlmItemGrantProposal {
        target_entity_id: target,
        item_id: "item_gold_coin".to_string(),
        quantity: 999_999,
        source: "llm_free_wealth".to_string(),
    };

    let result_excess = validate_and_grant_item(&registry, &excess_proposal);
    assert!(
        result_excess.is_err(),
        "Server verification must reject proposals exceeding authorized limits"
    );

    // 3. Legitimate registered item proposal
    let valid_proposal = LlmItemGrantProposal {
        target_entity_id: target,
        item_id: "item_bread".to_string(),
        quantity: 2,
        source: "quest_reward".to_string(),
    };

    let result_valid = validate_and_grant_item(&registry, &valid_proposal);
    assert_eq!(result_valid.unwrap(), 2);
}

#[test]
fn test_ai_006_memory_deduplication_via_source_event_id() {
    let mut memory_ref = MemoryRef::new();
    let shared_event_id = Uuid::new_v4();
    let target = EntityId::new();

    // 1. Initial absorption of the event memory
    let recorded_first = MemoryManager::record_event(
        &mut memory_ref,
        Some(shared_event_id),
        "GoblinAmbush",
        Some(target),
        800,
        10,
        300,
    );
    assert!(recorded_first, "First memory event must be accepted");
    assert_eq!(memory_ref.memories.len(), 1);

    // 2. Attempt duplicate absorption with identical source_event_id
    let recorded_duplicate = MemoryManager::record_event(
        &mut memory_ref,
        Some(shared_event_id),
        "GoblinAmbush_DuplicateStream",
        Some(target),
        800,
        12,
        300,
    );
    assert!(
        !recorded_duplicate,
        "Duplicate source_event_id must be rejected (0 duplicate absorbed) (TEST-AI-006)"
    );
    assert_eq!(
        memory_ref.memories.len(),
        1,
        "Memory count must strictly remain 1"
    );

    // 3. Different source_event_id should be accepted
    let recorded_different = MemoryManager::record_event(
        &mut memory_ref,
        Some(Uuid::new_v4()),
        "TradeSuccess",
        Some(target),
        200,
        15,
        300,
    );
    assert!(
        recorded_different,
        "Unique source_event_id must be accepted"
    );
    assert_eq!(memory_ref.memories.len(), 2);

    // 4. Memory TTL pruning check: at tick 312, event 1 (10+300=310) is expired, event 2 (15+300=315) is active
    MemoryManager::prune_expired(&mut memory_ref, 312);
    assert_eq!(memory_ref.memories.len(), 1);
    assert_eq!(memory_ref.memories[0].kind, "TradeSuccess");
}

#[test]
fn test_dod_10_npcs_independent_routine_and_threat_evasion() {
    let ai_system = NpcAiSystem::default();
    let mut npcs: Vec<(NpcAgentState, Needs, Personality, Position)> = (0..10)
        .map(|i| {
            let id = EntityId::new();
            let agent = NpcAgentState::new(id);
            let needs = Needs {
                hunger: (i * 80) as u16,
                fatigue: (i * 60) as u16,
                safety: 1000,
            };
            let personality = Personality {
                courage: (i * 50) as i16,
                greed: 100,
                loyalty: 50,
            };
            let pos = Position {
                x_millitile: (i as i64) * 2000,
                y_millitile: 0,
            };
            (agent, needs, personality, pos)
        })
        .collect();

    let mut events = Vec::new();

    // Peaceful routine step
    for (agent, needs, personality, pos) in &mut npcs {
        let ctx = PerceptionContext {
            self_entity_id: agent.entity_id,
            self_pos: *pos,
            self_faction: None,
            inventory: None,
            market_pos: Some(Position {
                x_millitile: 5000,
                y_millitile: 5000,
            }),
            home_pos: Some(Position {
                x_millitile: 0,
                y_millitile: 0,
            }),
            region_id: game_domain::RegionId::new(),
            session_id: game_domain::SessionId::from_u128(0),
        };

        ai_system.step_npc(agent, &ctx, &[], needs, personality, 1, &mut events);
        assert!(agent.current_goal.is_some());
    }

    // Now introduce an aggressive monster within range of NPC 0
    let monster_pos = Position {
        x_millitile: 500,
        y_millitile: 500,
    };
    let monster_id = EntityId::new();
    let monster_combat = CombatStats::new(1500, 20, 2, 4);

    let threat_entity = [(
        monster_id,
        monster_pos,
        None,
        Some(monster_combat),
        Some(MonsterArchetype::OrcWarrior),
    )];

    let (agent0, needs0, personality0, pos0) = &mut npcs[0];
    let ctx0 = PerceptionContext {
        self_entity_id: agent0.entity_id,
        self_pos: *pos0,
        self_faction: None,
        inventory: None,
        market_pos: None,
        home_pos: None,
        region_id: game_domain::RegionId::new(),
        session_id: game_domain::SessionId::from_u128(0),
    };

    ai_system.step_npc(
        agent0,
        &ctx0,
        &threat_entity,
        needs0,
        personality0,
        2,
        &mut events,
    );

    assert_eq!(
        agent0.current_goal,
        Some(NpcGoal::Flee),
        "NPC 0 must immediately switch to FLEE when monster threatens"
    );
}

#[test]
fn test_dod_1000_npcs_distributed_scheduling_strictly_obeys_budget() {
    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 10,
        max_evaluations_per_tick: 150,
    });

    let entity_ids: Vec<EntityId> = (0..1000).map(|_| EntityId::new()).collect();

    let mut total_scheduled = 0;
    for tick in 0..10 {
        let batch = scheduler.schedule_batch(&entity_ids, tick);
        assert!(
            batch.len() <= 150,
            "Per-tick evaluation budget (150) must never be exceeded (got {})",
            batch.len()
        );
        total_scheduled += batch.len();
    }

    // Over 10 ticks with 10-tick interval, approximately all 1,000 NPCs are serviced evenly
    assert!(
        total_scheduled >= 800,
        "Total scheduled ({total_scheduled}) across 10 ticks should cover the population"
    );
}

#[test]
fn test_authoritative_state_applied_during_execution() {
    use ecs_components::{AIState, RegionWorld};
    use game_domain::{
        EntityStateDto, InventoryProjectionDto, InventorySlotDto, NeedsDto, PersonalityDto,
        PositionDto, RegionId,
    };
    use std::collections::BTreeMap;

    let ai_system = NpcAiSystem::default();
    let region_id = RegionId::new();
    let mut world = RegionWorld::new_default_64x64(region_id, 1);

    let npc_id = EntityId::new();
    let initial_pos = Position {
        x_millitile: 5000,
        y_millitile: 5000,
    };
    let personality = Personality {
        courage: 0,
        greed: 100,
        loyalty: 0,
    };

    world
        .spawn_entity(EntityStateDto {
            id: npc_id,
            position: PositionDto {
                x_millitile: 5000,
                y_millitile: 5000,
            },
            velocity: None,
            health: None,
            needs: Some(NeedsDto {
                hunger: 800,
                fatigue: 100,
                safety: 1000,
            }),
            personality: Some(PersonalityDto {
                courage: 0,
                greed: 100,
                loyalty: 0,
            }),
            inventory: Some(InventoryProjectionDto {
                committed_version: 1,
                slots: vec![InventorySlotDto {
                    slot_index: 0,
                    item_id: "gold_coin".to_string(),
                    quantity: 10,
                }],
            }),
            faction: None,
            collider: None,
        })
        .expect("Entity spawn must succeed");

    // Add default AIState component to spawned entity
    let bevy_entity = world.get_entity(npc_id).expect("Entity must exist");
    world
        .world
        .entity_mut(bevy_entity)
        .insert(AIState::default());

    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 1,
        max_evaluations_per_tick: 10,
    });
    let mut agents = BTreeMap::new();
    let mut events = Vec::new();
    let mut commands = Vec::new();
    let authoritative_session_id = game_domain::SessionId::new();
    let command_outcomes = BTreeMap::new();

    // Step world simulation
    ai_system.step_simulation_world(
        &mut world,
        &scheduler,
        &mut agents,
        1,
        authoritative_session_id,
        &mut events,
        &mut commands,
        &command_outcomes,
    );

    // Verify commands were generated and enqueued with authoritative region & session metadata
    assert!(
        !commands.is_empty(),
        "Authoritative commands must be generated during plan step execution"
    );
    assert_eq!(
        commands[0].region_id, world.meta.region_id,
        "Command region_id must match authoritative RegionWorld"
    );
    assert_eq!(
        commands[0].session_id, authoritative_session_id,
        "Command session_id must match authoritative session"
    );
    assert_eq!(
        commands[0].actor_id, npc_id,
        "Command actor_id must match executing NPC"
    );

    // Verify AIState in Bevy ECS was authoritatively updated
    let bevy_entity = world.get_entity(npc_id).expect("Entity must exist");
    let ai_state = world
        .world
        .entity(bevy_entity)
        .get::<AIState>()
        .expect("AIState must exist");
    assert!(!ai_state.goal.is_empty(), "AIState goal must be populated");

    // Test direct action effect on Needs
    let test_needs = Needs {
        hunger: 900,
        fatigue: 600,
        safety: 1000,
    };
    let mut agent = NpcAgentState::new(npc_id);
    agent.current_goal = Some(NpcGoal::Eat);
    agent.current_plan = vec![GoapActionId::EatFood];
    agent.step_index = 0;
    agent.lifecycle = NpcAiLifecycleState::Executing;

    let food_inv = ecs_components::InventoryProjection {
        committed_version: 1,
        slots: vec![InventorySlotDto {
            slot_index: 0,
            item_id: "bread_food".to_string(),
            quantity: 5,
        }],
    };

    let ctx = PerceptionContext {
        self_entity_id: npc_id,
        self_pos: initial_pos,
        self_faction: None,
        inventory: Some(&food_inv),
        market_pos: None,
        home_pos: None,
        region_id: game_domain::RegionId::new(),
        session_id: game_domain::SessionId::from_u128(0),
    };

    // Simulate EatFood execution with has_food = true
    let (_ai, cmd) = ai_system.step_npc(
        &mut agent,
        &ctx,
        &[],
        &test_needs,
        &personality,
        2,
        &mut events,
    );

    let cmd = cmd.expect("EatFood must produce command envelope");
    assert_eq!(
        agent.pending_command.map(|p| p.command_id),
        Some(cmd.command_id),
        "Agent must track in-flight pending command"
    );

    // Authoritative execution reconciliation:
    // When engine reports success, agent advances step_index to 1 and completes plan
    agent.reconcile_command_outcome(cmd.command_id, true, 2, &mut events);
    assert_eq!(
        agent.step_index, 1,
        "Authoritative execution success must advance step_index"
    );
    assert_eq!(
        agent.lifecycle,
        NpcAiLifecycleState::Success,
        "Single-step plan must transition to Success upon authoritative completion"
    );
    assert!(agent.pending_command.is_none());
}

#[test]
fn test_scheduler_zero_budget_schedules_no_entities() {
    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 1,
        max_evaluations_per_tick: 0,
    });
    let entity_ids = vec![EntityId::new(), EntityId::new()];
    let scheduled = scheduler.schedule_batch(&entity_ids, 0);
    assert!(
        scheduled.is_empty(),
        "Zero evaluation budget must schedule 0 entities"
    );
}

#[test]
fn test_scheduler_fair_bounded_coverage_under_oversubscribed_budget() {
    use std::collections::HashSet;

    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 0,
        max_evaluations_per_tick: 200,
    });

    let entity_ids: Vec<EntityId> = (0..1000).map(|_| EntityId::new()).collect();
    let mut covered: HashSet<EntityId> = HashSet::new();

    // With 1000 entities and budget 200, exactly 5 ticks must cover all 1000 entities without starvation
    for tick in 0..5 {
        let batch = scheduler.schedule_batch(&entity_ids, tick);
        assert_eq!(batch.len(), 200, "Tick must fill max budget of 200");
        for id in batch {
            covered.insert(id);
        }
    }

    assert_eq!(
        covered.len(),
        1000,
        "All 1,000 NPCs must be scheduled within 5 ticks under rotating fair budget"
    );
}

#[test]
fn test_scheduler_input_order_independence_and_determinism() {
    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 0,
        max_evaluations_per_tick: 50,
    });

    let mut entity_ids: Vec<EntityId> = (0..100).map(|_| EntityId::new()).collect();
    let batch_forward = scheduler.schedule_batch(&entity_ids, 0);

    // Reverse input slice
    entity_ids.reverse();
    let batch_reversed = scheduler.schedule_batch(&entity_ids, 0);

    assert_eq!(
        batch_forward, batch_reversed,
        "Canonical sorting must ensure identical scheduling outcome regardless of caller slice ordering"
    );
}

#[test]
fn test_ai_commands_authoritatively_admitted_and_executed_by_engine() {
    use ecs_components::RegionWorld;
    use game_domain::{RegionId, SessionId};
    use simulation_core::command::DeduplicationOutcome;
    use simulation_core::engine::SimulationEngine;
    use std::collections::BTreeMap;

    let region_id = RegionId::new();
    let mut world = RegionWorld::new(region_id, 1, 100, 100);
    let entity_id = EntityId::new();
    let session_id = SessionId::from_u128(101);

    world
        .spawn_entity(game_domain::state::EntityStateDto {
            id: entity_id,
            position: game_domain::state::PositionDto {
                x_millitile: 5000,
                y_millitile: 5000,
            },
            velocity: Some(game_domain::state::VelocityDto { dx: 0, dy: 0 }),
            health: Some(game_domain::state::HealthDto {
                current: 100,
                max: 100,
                status: game_domain::state::CombatStatus::Alive,
            }),
            needs: Some(game_domain::state::NeedsDto {
                hunger: 900,
                fatigue: 100,
                safety: 1000,
            }),
            personality: Some(game_domain::state::PersonalityDto {
                courage: 50,
                greed: 50,
                loyalty: 50,
            }),
            inventory: Some(game_domain::state::InventoryProjectionDto {
                committed_version: 1,
                slots: vec![game_domain::state::InventorySlotDto {
                    slot_index: 0,
                    item_id: "gold".to_string(),
                    quantity: 100,
                }],
            }),
            faction: None,
            collider: None,
        })
        .expect("actor spawned");

    let ai_system = NpcAiSystem::default();
    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 1,
        max_evaluations_per_tick: 50,
    });
    let mut agents = std::collections::BTreeMap::new();
    let mut events = Vec::new();
    let mut commands = Vec::new();
    let mut command_outcomes = std::collections::BTreeMap::new();

    // Step AI system to generate plan and commands
    ai_system.step_simulation_world(
        &mut world,
        &scheduler,
        &mut agents,
        1,
        session_id,
        &mut events,
        &mut commands,
        &command_outcomes,
    );

    assert!(
        !commands.is_empty(),
        "AI system must produce command envelopes for scheduled actions"
    );

    // Create authoritative simulation engine with world and execute commands
    let mut engine = SimulationEngine::new(world, 1337);

    for cmd in &commands {
        assert_eq!(
            cmd.session_id, session_id,
            "Command session_id must match authoritative session"
        );
        let outcome = engine
            .submit_envelope(cmd.clone())
            .expect("Command envelope must be schema-valid and admitted");
        assert_eq!(
            outcome,
            DeduplicationOutcome::New,
            "New AI command envelope must pass deduplication"
        );
    }

    // Tick 1 advances clock to 1
    engine
        .step()
        .expect("Simulation tick 1 execution must succeed");

    // Tick 2 drains and executes commands scheduled for assigned_tick 2
    let tick_output = engine
        .step()
        .expect("Simulation tick 2 execution must succeed");
    assert_eq!(
        tick_output.tick, 2,
        "Authoritative simulation tick must advance to 2"
    );

    // Collect authoritative command execution outcomes from engine
    for cmd in &commands {
        if let Some(status) = engine.get_command_status(cmd.command_id) {
            command_outcomes.insert(cmd.command_id, status);
        }
    }

    // Step AI system again on tick 2 to reconcile authoritative engine outcomes
    let mut next_commands = Vec::new();
    ai_system.step_simulation_world(
        &mut engine.world,
        &scheduler,
        &mut agents,
        2,
        session_id,
        &mut events,
        &mut next_commands,
        &command_outcomes,
    );

    // Verify agent authoritatively advanced past the first step after Succeeded outcome
    let agent_after = agents.get(&entity_id).expect("agent must exist");
    assert_eq!(
        agent_after.step_index, 1,
        "Agent must advance plan step_index only after authoritative Succeeded outcome"
    );

    // Verify gold was deducted (100 - 10 = 90) and food was granted by the AI's emitted Purchase command
    let bevy_entity = engine.world.get_entity(entity_id).expect("entity exists");
    let inv = engine
        .world
        .world
        .entity(bevy_entity)
        .get::<ecs_components::InventoryProjection>()
        .expect("inv exists");
    let gold_slot = inv
        .slots
        .iter()
        .find(|s| s.item_id.contains("gold"))
        .expect("gold slot exists");
    assert_eq!(
        gold_slot.quantity, 90,
        "Authoritative engine must deduct 10 gold"
    );
    let food_slot = inv
        .slots
        .iter()
        .find(|s| s.item_id.contains("food"))
        .expect("food slot exists");
    assert_eq!(
        food_slot.quantity, 1,
        "Authoritative engine must grant 1 food item"
    );

    // 2. Authoritative engine verification for the AI's next plan step: npc_eat_food
    assert_eq!(
        next_commands.len(),
        1,
        "AI system on tick 2 must emit npc_eat_food command envelope"
    );
    let next_cmd = &next_commands[0];
    let outcome = engine
        .submit_envelope(next_cmd.clone())
        .expect("npc_eat_food admitted");
    assert_eq!(outcome, DeduplicationOutcome::New);

    let tick_3_output = engine.step().expect("Eat tick executed");
    assert_eq!(tick_3_output.tick, 3);

    // Verify hunger decreased by 400 (900 -> 500) and food quantity decreased by 1 (1 -> 0)
    let entity_ref = engine.world.world.entity(bevy_entity);
    let needs = entity_ref
        .get::<ecs_components::Needs>()
        .expect("needs exists");
    assert_eq!(
        needs.hunger, 500,
        "Authoritative engine must reduce hunger by 400 upon eating"
    );
    let inv_after = entity_ref
        .get::<ecs_components::InventoryProjection>()
        .expect("inv exists");
    let food_after = inv_after
        .slots
        .iter()
        .find(|s| s.item_id.contains("food"))
        .expect("food slot exists");
    assert_eq!(
        food_after.quantity, 0,
        "Authoritative engine must consume 1 food item"
    );

    // 3. Final reconciliation pass: AI receives Succeeded status for npc_eat_food and completes the plan
    let mut final_outcomes = BTreeMap::new();
    if let Some(status) = engine.get_command_status(next_cmd.command_id) {
        final_outcomes.insert(next_cmd.command_id, status);
    }
    let mut empty_commands = Vec::new();
    ai_system.step_simulation_world(
        &mut engine.world,
        &scheduler,
        &mut agents,
        3,
        session_id,
        &mut events,
        &mut empty_commands,
        &final_outcomes,
    );

    let plan_completed = events.iter().any(|e| match e {
        SimulationEvent::NpcPlanCompleted {
            entity_id: id,
            goal,
            ..
        } => *id == entity_id && goal == "Eat",
        _ => false,
    });
    assert!(
        plan_completed,
        "Agent must emit NpcPlanCompleted upon authoritative execution of the Eat plan"
    );
}

#[test]
fn test_reconciliation_fails_and_falls_back_on_command_rejection() {
    use game_domain::CommandId;
    use simulation_core::command::CommandStatus;

    let ai_system = NpcAiSystem::default();
    let scheduler = NpcScheduler::new(NpcSchedulerConfig {
        decision_interval_ticks: 1,
        max_evaluations_per_tick: 50,
    });
    let npc_id = EntityId::new();
    let mut agent = NpcAgentState::new(npc_id);
    let mut agents = std::collections::BTreeMap::new();
    let mut events = Vec::new();
    let mut commands = Vec::new();

    let region_id = game_domain::RegionId::new();
    let mut world = ecs_components::RegionWorld::new(region_id, 1, 100, 100);
    world
        .spawn_entity(game_domain::state::EntityStateDto {
            id: npc_id,
            position: game_domain::state::PositionDto {
                x_millitile: 0,
                y_millitile: 0,
            },
            velocity: None,
            health: None,
            needs: Some(game_domain::state::NeedsDto {
                hunger: 900,
                fatigue: 0,
                safety: 1000,
            }),
            personality: Some(game_domain::state::PersonalityDto {
                courage: 50,
                greed: 50,
                loyalty: 50,
            }),
            inventory: Some(game_domain::state::InventoryProjectionDto {
                committed_version: 1,
                slots: vec![game_domain::state::InventorySlotDto {
                    slot_index: 0,
                    item_id: "gold".to_string(),
                    quantity: 100,
                }],
            }),
            faction: None,
            collider: None,
        })
        .expect("spawned");

    agent.current_plan = vec![GoapActionId::MoveToMarket];
    agent.step_index = 0;
    agent.lifecycle = NpcAiLifecycleState::Executing;
    agent.pending_command = Some(simulation_systems::system::NpcPendingCommand {
        command_id: CommandId::from_u128(777),
        action: GoapActionId::MoveToMarket,
        plan_index: 0,
        issued_tick: 1,
    });
    agents.insert(npc_id, agent);

    // Simulate authoritative engine rejection (CommandStatus::Failed)
    let mut outcomes = std::collections::BTreeMap::new();
    outcomes.insert(CommandId::from_u128(777), CommandStatus::Failed);

    ai_system.step_simulation_world(
        &mut world,
        &scheduler,
        &mut agents,
        2,
        game_domain::SessionId::from_u128(1),
        &mut events,
        &mut commands,
        &outcomes,
    );

    let agent_after = agents.get(&npc_id).expect("agent exists");
    assert_eq!(
        agent_after.consecutive_failures, 1,
        "Command failure must increment consecutive_failures"
    );
    assert_eq!(
        agent_after.step_index, 0,
        "Failed command must not advance step_index"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SimulationEvent::NpcPlanFailed { .. })),
        "NpcPlanFailed event must be emitted on authoritative command failure"
    );
}

#[test]
fn test_urgent_threat_cancels_pending_command_and_discards_superseded_outcome() {
    use game_domain::CommandId;

    let ai_system = NpcAiSystem::default();
    let npc_id = EntityId::new();
    let mut agent = NpcAgentState::new(npc_id);
    let mut events = Vec::new();

    // Agent is executing work, with a pending command in-flight
    let old_cmd_id = CommandId::from_u128(5555);
    agent.current_goal = Some(NpcGoal::Work);
    agent.current_plan = vec![GoapActionId::WorkField];
    agent.step_index = 0;
    agent.pending_command = Some(simulation_systems::system::NpcPendingCommand {
        command_id: old_cmd_id,
        action: GoapActionId::WorkField,
        plan_index: 0,
        issued_tick: 1,
    });

    let ctx = PerceptionContext {
        self_entity_id: npc_id,
        self_pos: Position {
            x_millitile: 1000,
            y_millitile: 1000,
        },
        self_faction: None,
        inventory: None,
        market_pos: None,
        home_pos: None,
        region_id: game_domain::RegionId::new(),
        session_id: game_domain::SessionId::from_u128(0),
    };
    let personality = Personality {
        courage: 0,
        greed: 0,
        loyalty: 0,
    };
    let needs = Needs {
        hunger: 100,
        fatigue: 100,
        safety: 1000,
    };

    // Imminent threat appears
    let monster_pos = Position {
        x_millitile: 1200,
        y_millitile: 1200,
    };
    let threat_entity = [(
        EntityId::new(),
        monster_pos,
        None,
        Some(CombatStats::new(1000, 20, 1, 3)),
        Some(MonsterArchetype::OrcWarrior),
    )];

    let (_ai, _cmd) = ai_system.step_npc(
        &mut agent,
        &ctx,
        &threat_entity,
        &needs,
        &personality,
        2,
        &mut events,
    );

    // Urgent threat must interrupt plan and switch to Flee
    assert_eq!(
        agent.current_goal,
        Some(NpcGoal::Flee),
        "Urgent threat must immediately interrupt goal and switch to Flee"
    );
    assert!(
        agent.superseded_commands.contains(&old_cmd_id),
        "Pending command must be recorded in superseded_commands on threat interrupt"
    );

    // Now suppose the old superseded command completes on the engine
    agent.reconcile_command_outcome(old_cmd_id, true, 2, &mut events);

    // The outcome from the superseded command must be ignored and not advance the flee plan
    assert_eq!(
        agent.step_index, 0,
        "Superseded command outcome must be discarded without advancing the new flee plan"
    );
    assert_eq!(agent.current_goal, Some(NpcGoal::Flee));
}
