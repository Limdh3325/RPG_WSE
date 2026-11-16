#![allow(clippy::collapsible_if)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ecs_components::region_world::RegionWorld;
use game_domain::{
    errors::DomainError,
    id::{CommandId, EntityId, RegionId, SessionId},
    state::{CombatStatus, EntityStateDto, HealthDto, PositionDto, VelocityDto},
};
use simulation_core::{
    SimulationEngine,
    command::{
        CommandEnvelope, CommandPayload, CommandStatus, DeduplicationOutcome, IngressCommand,
        SUPPORTED_COMMAND_SCHEMA_VERSION, TransientCommandDeduplicator,
    },
    event::{
        EventBudget, EventEnvelope, EventRule, RuleAction, SUPPORTED_EVENT_SCHEMA_VERSION,
        SimulationEvent,
    },
};
use uuid::Uuid;

fn setup_test_engine(seed: u64) -> (SimulationEngine, EntityId) {
    let region_id = RegionId::from_u128(seed as u128);
    let mut world = RegionWorld::new_default_64x64(region_id, 1);
    let player = EntityId::from_u128(1);
    let entity_dto = EntityStateDto {
        id: player,
        position: PositionDto {
            x_millitile: 10_000,
            y_millitile: 10_000,
        },
        velocity: Some(VelocityDto { dx: 0, dy: 0 }),
        health: Some(HealthDto {
            current: 100,
            max: 100,
            status: CombatStatus::Alive,
        }),
        needs: None,
        personality: None,
        inventory: None,
        faction: None,
        collider: None,
    };
    world.spawn_entity(entity_dto).expect("spawn entity");
    let engine = SimulationEngine::new(world, seed);
    (engine, player)
}

// -----------------------------------------------------------------------------
// TEST-CMD-001: Transient deduplication: 100 identical purchase commands -> 1 execution
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_001_transient_deduplication_single_execution() {
    let (mut engine, player) = setup_test_engine(42);
    let command_id = CommandId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    let payload = CommandPayload::Purchase {
        item_id: 1001,
        price: 50,
    };
    let envelope = CommandEnvelope::new(
        command_id, player, region_id, session_id, 1, 1, 1, 0, payload,
    );

    // Submit identical command 100 times
    let mut new_count = 0;
    let mut duplicate_count = 0;

    for _ in 0..100 {
        match engine.submit_envelope(envelope.clone()).unwrap() {
            DeduplicationOutcome::New => new_count += 1,
            DeduplicationOutcome::Duplicate { .. } => duplicate_count += 1,
        }
    }

    assert_eq!(new_count, 1, "Exactly 1 command should be marked as New");
    assert_eq!(
        duplicate_count, 99,
        "Remaining 99 commands must be marked as Duplicate"
    );

    // Step tick 1: exactly 1 PurchaseExecuted event must be emitted
    let output = engine.step().unwrap();
    let purchase_events: Vec<_> = output
        .events
        .iter()
        .filter(|e| matches!(e, SimulationEvent::PurchaseExecuted { .. }))
        .collect();

    assert_eq!(
        purchase_events.len(),
        1,
        "Exactly 1 PurchaseExecuted event must occur despite 100 submissions"
    );
}

// -----------------------------------------------------------------------------
// TEST-CMD-002: Same command_id with different payload rejected with IdempotencyConflict
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_002_idempotency_conflict_rejection() {
    let (mut engine, player) = setup_test_engine(42);
    let command_id = CommandId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    let cmd_a = CommandEnvelope::new(
        command_id,
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 12_000,
            target_y: 12_000,
        },
    );

    let cmd_b = CommandEnvelope::new(
        command_id,
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 20_000,
            target_y: 20_000,
        },
    );

    assert_eq!(
        engine.submit_envelope(cmd_a).unwrap(),
        DeduplicationOutcome::New
    );

    let err = engine.submit_envelope(cmd_b).unwrap_err();
    match err {
        DomainError::IdempotencyConflict {
            command_id: err_cmd_id,
            existing_hash,
            incoming_hash,
        } => {
            assert_eq!(err_cmd_id, command_id.0);
            assert_ne!(existing_hash, incoming_hash);
        }
        other => panic!("Expected IdempotencyConflict, got: {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// TEST-CMD-003: Deterministic ordering across out-of-order submissions
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_003_reproducible_ordering_across_runs() {
    let run = |seed: u64| -> (String, Vec<SimulationEvent>) {
        let (mut engine, player) = setup_test_engine(seed);
        let region_id = engine.world.meta.region_id;
        let session_id = SessionId::from_u128(999);

        let cmd1 = CommandEnvelope::new(
            CommandId::from_u128(101),
            player,
            region_id,
            session_id,
            1,
            1,
            1,
            0,
            CommandPayload::Move {
                target_x: 11_000,
                target_y: 10_000,
            },
        );
        let cmd2 = CommandEnvelope::new(
            CommandId::from_u128(102),
            player,
            region_id,
            session_id,
            2,
            1,
            2,
            0,
            CommandPayload::Move {
                target_x: 12_000,
                target_y: 10_000,
            },
        );
        let cmd3 = CommandEnvelope::new(
            CommandId::from_u128(103),
            player,
            region_id,
            session_id,
            3,
            1,
            3,
            0,
            CommandPayload::Move {
                target_x: 13_000,
                target_y: 10_000,
            },
        );

        // Submit in reverse arrival order: 3, 1, 2
        engine.submit_envelope(cmd3).unwrap();
        engine.submit_envelope(cmd1).unwrap();
        engine.submit_envelope(cmd2).unwrap();

        let output = engine.step().unwrap();
        (output.state_hash, output.events)
    };

    let (hash_a, events_a) = run(100);
    let (hash_b, events_b) = run(100);

    assert_eq!(
        hash_a, hash_b,
        "Two independent runs must produce identical state hash"
    );
    assert_eq!(
        events_a, events_b,
        "Events must be identical in content and order"
    );
}

// -----------------------------------------------------------------------------
// TEST-CMD-004: Schema version validation
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_004_unsupported_schema_version_rejected() {
    let (mut engine, player) = setup_test_engine(42);
    let mut envelope = CommandEnvelope::new(
        CommandId::new(),
        player,
        engine.world.meta.region_id,
        SessionId::new(),
        1,
        1,
        1,
        0,
        CommandPayload::Noop,
    );
    envelope.schema_version = 999;

    let err = engine.submit_envelope(envelope).unwrap_err();
    assert_eq!(
        err,
        DomainError::InvalidSchemaVersion {
            supported: SUPPORTED_COMMAND_SCHEMA_VERSION,
            received: 999,
        }
    );
}

// -----------------------------------------------------------------------------
// TEST-CMD-005: State machine lifecycle transitions
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_005_status_lifecycle_transitions() {
    let mut status = CommandStatus::Received;
    assert!(status.can_transition_to(CommandStatus::Accepted));
    assert!(status.can_transition_to(CommandStatus::Rejected));
    assert!(!status.can_transition_to(CommandStatus::Succeeded));

    status.transition_to(CommandStatus::Accepted).unwrap();
    assert_eq!(status, CommandStatus::Accepted);

    status.transition_to(CommandStatus::PendingCommit).unwrap();
    assert_eq!(status, CommandStatus::PendingCommit);

    status.transition_to(CommandStatus::Succeeded).unwrap();
    assert_eq!(status, CommandStatus::Succeeded);

    // Terminal state cannot transition
    assert!(status.transition_to(CommandStatus::Accepted).is_err());
}

// -----------------------------------------------------------------------------
// TEST-CMD-006: Forged payload hash rejection
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_006_forged_payload_hash_rejected() {
    let (mut engine, player) = setup_test_engine(42);
    let mut envelope = CommandEnvelope::new(
        CommandId::new(),
        player,
        engine.world.meta.region_id,
        SessionId::new(),
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 15_000,
            target_y: 15_000,
        },
    );
    // Tamper with payload_hash
    envelope.payload_hash = [0xFF; 32];

    let err = engine.submit_envelope(envelope).unwrap_err();
    match err {
        DomainError::InvalidCommand { reason } => {
            assert!(
                reason.contains("payload hash mismatch"),
                "Expected hash mismatch reason, got: {reason}"
            );
        }
        other => panic!("Expected InvalidCommand error, got: {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// TEST-CMD-007: 128-bit CommandId collision safety (no 64-bit truncation collision)
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_007_128bit_id_no_truncation_collision() {
    let (mut engine, player) = setup_test_engine(42);
    let region_id = engine.world.meta.region_id;
    let session_id = SessionId::new();

    // Two command IDs sharing the same lower 64 bits (0x1234), but differing in high 64 bits
    let id_a = CommandId(Uuid::from_u128(0x0000_0001_0000_0000_0000_0000_0000_1234));
    let id_b = CommandId(Uuid::from_u128(0x0000_0002_0000_0000_0000_0000_0000_1234));

    let cmd_a = CommandEnvelope::new(
        id_a,
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 11_000,
            target_y: 10_000,
        },
    );

    let cmd_b = CommandEnvelope::new(
        id_b,
        player,
        region_id,
        session_id,
        2,
        1,
        1, // Same ingress_seq to test tie-breaking on full 128-bit command_id
        0,
        CommandPayload::Move {
            target_x: 12_000,
            target_y: 10_000,
        },
    );

    assert_eq!(
        engine.submit_envelope(cmd_a).unwrap(),
        DeduplicationOutcome::New
    );
    assert_eq!(
        engine.submit_envelope(cmd_b).unwrap(),
        DeduplicationOutcome::New,
        "Distinct 128-bit IDs must not collide even if low 64 bits match"
    );

    let output = engine.step().unwrap();
    assert_eq!(output.tick, 1);
}

// -----------------------------------------------------------------------------
// TEST-CMD-008: Command lifecycle status transitions during actual execution
// -----------------------------------------------------------------------------
#[test]
fn test_cmd_008_lifecycle_execution_transitions() {
    let (mut engine, player) = setup_test_engine(42);
    let region_id = engine.world.meta.region_id;
    let session_id = SessionId::new();

    let valid_cmd_id = CommandId::new();
    let valid_cmd = CommandEnvelope::new(
        valid_cmd_id,
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 11_000,
            target_y: 10_000,
        },
    );

    let invalid_cmd_id = CommandId::new();
    let invalid_cmd = CommandEnvelope::new(
        invalid_cmd_id,
        player,
        region_id,
        session_id,
        2,
        1,
        2,
        0,
        CommandPayload::Move {
            target_x: 999_999, // Out of bounds
            target_y: 999_999,
        },
    );

    engine.submit_envelope(valid_cmd).unwrap();
    engine.submit_envelope(invalid_cmd).unwrap();

    // Prior to tick execution, both are Accepted
    assert_eq!(
        engine.deduplicator.get_status(valid_cmd_id),
        Some(CommandStatus::Accepted)
    );
    assert_eq!(
        engine.deduplicator.get_status(invalid_cmd_id),
        Some(CommandStatus::Accepted)
    );

    // Step tick 1
    let _ = engine.step().unwrap();

    // After execution: valid command is Succeeded, out-of-bounds command is Failed
    assert_eq!(
        engine.deduplicator.get_status(valid_cmd_id),
        Some(CommandStatus::Succeeded),
        "Valid executed command must transition to Succeeded"
    );
    assert_eq!(
        engine.deduplicator.get_status(invalid_cmd_id),
        Some(CommandStatus::Failed),
        "Rejected command must transition to Failed"
    );
}

// -----------------------------------------------------------------------------
// TEST-EVENT-001: Condition met once -> single event emitted
// -----------------------------------------------------------------------------
#[test]
fn test_event_001_single_event_emission() {
    let (mut engine, _player) = setup_test_engine(42);

    struct ShortageRule {
        triggered: AtomicUsize,
    }

    impl EventRule for ShortageRule {
        fn name(&self) -> &str {
            "ShortageRule"
        }
        fn evaluate(&self, event: &EventEnvelope, _tick: u64) -> Vec<RuleAction> {
            if let SimulationEvent::Custom { name, .. } = &event.payload {
                if name == "CHECK_SUPPLY" {
                    self.triggered.fetch_add(1, Ordering::SeqCst);
                    return vec![RuleAction::EmitEvent(SimulationEvent::FoodShortage {
                        settlement_id: None,
                        deficit: 10,
                    })];
                }
            }
            vec![]
        }
    }

    let rule = Arc::new(ShortageRule {
        triggered: AtomicUsize::new(0),
    });

    struct RuleWrapper(Arc<ShortageRule>);
    impl EventRule for RuleWrapper {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn evaluate(&self, event: &EventEnvelope, tick: u64) -> Vec<RuleAction> {
            self.0.evaluate(event, tick)
        }
    }

    engine.register_rule(Box::new(RuleWrapper(Arc::clone(&rule))));

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        EntityId::from_u128(1),
        engine.world.meta.region_id,
        SessionId::new(),
        1,
        1,
        1,
        0,
        CommandPayload::Custom {
            action: "CHECK_SUPPLY".to_string(),
            payload: vec![],
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let output = engine.step().unwrap();

    let shortage_events: Vec<_> = output
        .events
        .iter()
        .filter(|e| matches!(e, SimulationEvent::FoodShortage { .. }))
        .collect();

    assert_eq!(
        shortage_events.len(),
        1,
        "Exactly 1 food shortage event should be emitted"
    );
    assert_eq!(
        rule.triggered.load(Ordering::SeqCst),
        1,
        "Rule should only evaluate once"
    );
}

// -----------------------------------------------------------------------------
// TEST-EVENT-002: Cyclic rules stop cleanly at max_causation_depth
// -----------------------------------------------------------------------------
#[test]
fn test_event_002_cyclic_rules_cycle_detected() {
    let (mut engine, _player) = setup_test_engine(42);

    struct PingPongRule;
    impl EventRule for PingPongRule {
        fn name(&self) -> &str {
            "PingPongRule"
        }
        fn evaluate(&self, event: &EventEnvelope, _tick: u64) -> Vec<RuleAction> {
            match &event.payload {
                SimulationEvent::Custom { name, .. } if name == "PING" => {
                    vec![RuleAction::EmitEvent(SimulationEvent::Custom {
                        name: "PONG".to_string(),
                        data: "".to_string(),
                    })]
                }
                SimulationEvent::Custom { name, .. } if name == "PONG" => {
                    vec![RuleAction::EmitEvent(SimulationEvent::Custom {
                        name: "PING".to_string(),
                        data: "".to_string(),
                    })]
                }
                _ => vec![],
            }
        }
    }

    engine.register_rule(Box::new(PingPongRule));

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        EntityId::from_u128(1),
        engine.world.meta.region_id,
        SessionId::new(),
        1,
        1,
        1,
        0,
        CommandPayload::Custom {
            action: "PING".to_string(),
            payload: vec![],
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let output = engine.step().unwrap();

    let cycle_events: Vec<_> = output
        .events
        .iter()
        .filter(|e| matches!(e, SimulationEvent::RuleCycleDetected { .. }))
        .collect();

    assert!(
        !cycle_events.is_empty(),
        "RuleCycleDetected event must be emitted when cyclic rules recurse to max depth"
    );

    if let SimulationEvent::RuleCycleDetected { cycle_depth, .. } = cycle_events[0] {
        assert!(
            *cycle_depth >= 5,
            "Cycle depth must reach or exceed max_causation_depth"
        );
    }
}

// -----------------------------------------------------------------------------
// TEST-EVENT-003: Hard event budget limits ALL events emitted in a tick
// -----------------------------------------------------------------------------
#[test]
fn test_event_003_hard_event_budget_limits_all_events() {
    let (mut engine, _player) = setup_test_engine(42);
    // Strict small budget: max 5 events per tick
    engine.event_budget = EventBudget::new(5, 5);

    struct FloodRule;
    impl EventRule for FloodRule {
        fn name(&self) -> &str {
            "FloodRule"
        }
        fn evaluate(&self, event: &EventEnvelope, _tick: u64) -> Vec<RuleAction> {
            if let SimulationEvent::Custom { name, .. } = &event.payload {
                if name == "FLOOD" {
                    return (0..20)
                        .map(|i| {
                            RuleAction::EmitEvent(SimulationEvent::Custom {
                                name: format!("SPAM_{i}"),
                                data: "".to_string(),
                            })
                        })
                        .collect();
                }
            }
            vec![]
        }
    }

    engine.register_rule(Box::new(FloodRule));

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        EntityId::from_u128(1),
        engine.world.meta.region_id,
        SessionId::new(),
        1,
        1,
        1,
        0,
        CommandPayload::Custom {
            action: "FLOOD".to_string(),
            payload: vec![],
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let output = engine.step().unwrap();

    // Verify strict hard budget constraint
    assert!(
        output.events.len() <= 5,
        "Total events ({}) must never exceed max_events_per_tick (5)",
        output.events.len()
    );

    let overflow_events: Vec<_> = output
        .events
        .iter()
        .filter(|e| matches!(e, SimulationEvent::EventBudgetExceeded { .. }))
        .collect();

    assert_eq!(
        overflow_events.len(),
        1,
        "Expected exactly 1 EventBudgetExceeded event"
    );
}

// -----------------------------------------------------------------------------
// TEST-EVENT-004: Causation & correlation tracing preserves hierarchy
// -----------------------------------------------------------------------------
#[test]
fn test_event_004_causation_and_correlation_tracing() {
    let region_id = RegionId::new();
    let root_command_id = CommandId::new();

    let root_env = EventEnvelope::new_root(
        region_id,
        1,
        0,
        Some(root_command_id),
        None,
        SimulationEvent::Custom {
            name: "ROOT".to_string(),
            data: "".to_string(),
        },
    );
    let root_correlation_id = root_env.correlation_id;

    assert_eq!(root_env.causation_depth, 0);
    assert_eq!(root_env.causation_event_id, None);
    assert_eq!(root_env.correlation_id, root_correlation_id);
    assert_eq!(root_env.schema_version, SUPPORTED_EVENT_SCHEMA_VERSION);

    // Child 1
    let child_env1 = EventEnvelope::new_child(
        &root_env,
        1,
        0,
        SimulationEvent::Custom {
            name: "CHILD1".to_string(),
            data: "".to_string(),
        },
    );

    assert_eq!(child_env1.causation_depth, 1);
    assert_eq!(child_env1.causation_event_id, Some(root_env.event_id));
    assert_eq!(child_env1.correlation_id, root_correlation_id);

    // Child 2 (grandchild)
    let child_env2 = EventEnvelope::new_child(
        &child_env1,
        1,
        0,
        SimulationEvent::Custom {
            name: "CHILD2".to_string(),
            data: "".to_string(),
        },
    );

    assert_eq!(child_env2.causation_depth, 2);
    assert_eq!(child_env2.causation_event_id, Some(child_env1.event_id));
    assert_eq!(child_env2.correlation_id, root_correlation_id);
}

// -----------------------------------------------------------------------------
// TEST-EVENT-005: EventEnvelope deterministic replay (byte-for-byte reproducibility)
// -----------------------------------------------------------------------------
#[test]
fn test_event_005_deterministic_envelope_replay() {
    let run = |seed: u64| -> Vec<EventEnvelope> {
        let (mut engine, player) = setup_test_engine(seed);
        let cmd = CommandEnvelope::new(
            CommandId::from_u128(101),
            player,
            engine.world.meta.region_id,
            SessionId::from_u128(202),
            1,
            1,
            1,
            0,
            CommandPayload::Move {
                target_x: 11_000,
                target_y: 11_000,
            },
        );
        engine.submit_envelope(cmd).unwrap();
        let output = engine.step().unwrap();
        output.event_envelopes
    };

    let envelopes_run1 = run(777);
    let envelopes_run2 = run(777);

    assert_eq!(
        envelopes_run1.len(),
        envelopes_run2.len(),
        "Must have same event envelope count"
    );
    for (i, (e1, e2)) in envelopes_run1.iter().zip(envelopes_run2.iter()).enumerate() {
        assert_eq!(
            e1.event_id, e2.event_id,
            "Event {i} event_id must be byte-for-byte identical"
        );
        assert_eq!(
            e1.correlation_id, e2.correlation_id,
            "Event {i} correlation_id must be byte-for-byte identical"
        );
        assert_eq!(e1.tick, e2.tick, "Event {i} tick must match");
        assert_eq!(e1.payload, e2.payload, "Event {i} payload must match");
    }
}

// -----------------------------------------------------------------------------
// TEST-RULE-001: Subsequent commands scheduled for tick + 1 (delay >= 1)
// -----------------------------------------------------------------------------
#[test]
fn test_rule_001_followup_command_scheduled_next_tick() {
    let (mut engine, player) = setup_test_engine(42);
    let region_id = engine.world.meta.region_id;
    let session_id = SessionId::new();

    // Rule: when Custom event "TRIGGER_COMMAND" is received, schedule a Move command
    struct ScheduleRule {
        target_entity: EntityId,
        region_id: RegionId,
        session_id: SessionId,
    }

    impl EventRule for ScheduleRule {
        fn name(&self) -> &str {
            "ScheduleRule"
        }
        fn evaluate(&self, event: &EventEnvelope, _tick: u64) -> Vec<RuleAction> {
            if let SimulationEvent::Custom { name, .. } = &event.payload {
                if name == "TRIGGER_COMMAND" {
                    let cmd = CommandEnvelope::new(
                        CommandId::new(),
                        self.target_entity,
                        self.region_id,
                        self.session_id,
                        1,
                        1, // Intentionally requested at tick 1, but rule must enforce delay >= 1
                        1,
                        0,
                        CommandPayload::Move {
                            target_x: 15_000,
                            target_y: 10_000,
                        },
                    );
                    return vec![RuleAction::ScheduleFollowUpCommand {
                        delay_ticks: 1,
                        command: cmd,
                    }];
                }
            }
            vec![]
        }
    }

    engine.register_rule(Box::new(ScheduleRule {
        target_entity: player,
        region_id,
        session_id,
    }));

    let initial_cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Custom {
            action: "TRIGGER_COMMAND".to_string(),
            payload: vec![],
        },
    );
    engine.submit_envelope(initial_cmd).unwrap();

    // Tick 1: Custom event processed, follow-up command scheduled for tick 2
    let output1 = engine.step().unwrap();
    assert_eq!(output1.tick, 1);

    // Entity has not moved yet at tick 1
    let snapshot1 = engine.world.export_canonical();
    let p1 = snapshot1.entities.iter().find(|e| e.id == player).unwrap();
    assert_eq!(p1.position.x_millitile, 10_000);

    // Tick 2: scheduled command executes
    let output2 = engine.step().unwrap();
    assert_eq!(output2.tick, 2);

    let move_events: Vec<_> = output2
        .events
        .iter()
        .filter(|e| matches!(e, SimulationEvent::EntityMoved { .. }))
        .collect();
    assert_eq!(
        move_events.len(),
        1,
        "Move command should execute on tick 2"
    );
}

/// Regression test: EventBudget with max_events_per_tick == 0 must emit strictly 0 events and envelopes.
#[test]
fn test_event_budget_zero_capacity_strictly_empty() {
    let region_id = RegionId::from_u128(999);
    let world = RegionWorld::new(region_id, 1, 32, 32);
    let mut engine = SimulationEngine::new(world, 12345);
    engine.event_budget = EventBudget::new(0, 5);

    let output = engine.step().unwrap();
    assert_eq!(
        output.events.len(),
        0,
        "Zero-capacity event budget must result in 0 events in output"
    );
    assert_eq!(
        output.event_envelopes.len(),
        0,
        "Zero-capacity event budget must result in 0 envelopes in output"
    );
}

/// Regression test: Rule-generated child event provenance must be preserved in output event envelopes.
#[test]
fn test_rule_engine_child_provenance_preserved_in_output() {
    use simulation_core::event::{EventRule, RuleAction};

    struct ParentToChildRule;
    impl EventRule for ParentToChildRule {
        fn name(&self) -> &str {
            "ParentToChildRule"
        }

        fn evaluate(&self, event: &EventEnvelope, _current_tick: u64) -> Vec<RuleAction> {
            if let SimulationEvent::Custom { name, .. } = &event.payload {
                if name == "PARENT_EVENT" {
                    return vec![RuleAction::EmitEvent(SimulationEvent::Custom {
                        name: "CHILD_EVENT".to_string(),
                        data: "from_parent".to_string(),
                    })];
                }
            }
            Vec::new()
        }
    }

    let region_id = RegionId::from_u128(777);
    let world = RegionWorld::new(region_id, 1, 32, 32);
    let mut engine = SimulationEngine::new(world, 42);
    engine
        .rule_engine
        .register_rule(Box::new(ParentToChildRule));

    let player = EntityId::new();
    engine.spawn_actor(player, 10_000, 10_000, 100).unwrap();
    let session_id = SessionId::new();
    let parent_cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Custom {
            action: "PARENT_EVENT".to_string(),
            payload: vec![],
        },
    );
    engine.submit_envelope(parent_cmd).unwrap();

    let output = engine.step().unwrap();

    let parent_env = output
        .event_envelopes
        .iter()
        .find(|e| {
            matches!(&e.payload, SimulationEvent::Custom { name, .. } if name == "PARENT_EVENT")
        })
        .expect("Parent event envelope must be in output");

    let child_env = output
        .event_envelopes
        .iter()
        .find(
            |e| matches!(&e.payload, SimulationEvent::Custom { name, .. } if name == "CHILD_EVENT"),
        )
        .expect("Child event envelope must be in output");

    assert_eq!(
        child_env.causation_event_id,
        Some(parent_env.event_id),
        "Child envelope must reference parent event_id as causation_event_id"
    );
    assert_eq!(
        child_env.causation_depth,
        parent_env.causation_depth + 1,
        "Child causation depth must be parent depth + 1"
    );
    assert_eq!(
        child_env.correlation_id, parent_env.correlation_id,
        "Child correlation_id must match parent root correlation_id"
    );
}

/// Verification test: Ingress ordering, deduplication stress, and deterministic tick execution.
#[test]
fn test_cmd_ingress_stress_and_ordering_invariants() {
    let (mut engine, player) = setup_test_engine(42);
    let region_id = engine.world.meta.region_id;
    let session_id = SessionId::from_u128(888);

    // Create 10 commands with randomized arrival order and some duplicate submissions
    let mut commands = Vec::new();
    for i in 1..=10 {
        let cmd = CommandEnvelope::new(
            CommandId::from_u128(2000 + i),
            player,
            region_id,
            session_id,
            i as u64,
            1, // assigned_tick: 1
            i as u64,
            0,
            CommandPayload::Move {
                target_x: 10_000 + (i as i64 * 100),
                target_y: 10_000,
            },
        );
        commands.push(cmd);
    }

    // Submit out of order (reverse)
    for cmd in commands.iter().rev() {
        assert!(engine.submit_envelope(cmd.clone()).is_ok());
    }

    // Attempt to resubmit identical commands (transient duplicate) -> should succeed idempotently
    for cmd in commands.iter() {
        assert!(engine.submit_envelope(cmd.clone()).is_ok());
    }

    // Attempt to submit with conflicting payload for existing command_id -> must fail
    let conflict_cmd = CommandEnvelope::new(
        CommandId::from_u128(2001),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 99_999,
            target_y: 99_999,
        },
    );
    let submit_result = engine.submit_envelope(conflict_cmd);
    assert!(
        submit_result.is_err(),
        "Conflicting payload must be rejected"
    );

    // Execute tick 1
    let output = engine.step().unwrap();
    assert_eq!(output.tick, 1);

    // Verify deduplicator recorded exactly 10 unique commands
    assert_eq!(engine.deduplicator.len(), 10);
    for i in 1..=10 {
        assert_eq!(
            engine
                .deduplicator
                .get_status(CommandId::from_u128(2000 + i)),
            Some(CommandStatus::Succeeded)
        );
    }
}

/// Verification test: True LRU eviction behavior under capacity bound.
#[test]
fn test_cmd_deduplicator_true_lru_eviction() {
    let mut dedup = TransientCommandDeduplicator::new(3);
    let player = EntityId::from_u128(1);
    let region = RegionId::from_u128(1);

    let make_env = |id: u128, dx: i32| {
        CommandEnvelope::new(
            CommandId::from_u128(id),
            player,
            region,
            SessionId::from_u128(100),
            id as u64,
            1,
            id as u64,
            1,
            CommandPayload::SetVelocity { dx, dy: 0 },
        )
    };

    let env1 = make_env(1, 10);
    let env2 = make_env(2, 20);
    let env3 = make_env(3, 30);
    let env4 = make_env(4, 40);

    // Insert 1, 2, 3 -> buffer holds [1, 2, 3]
    assert_eq!(
        dedup.check_and_register(&env1).unwrap(),
        DeduplicationOutcome::New
    );
    assert_eq!(
        dedup.check_and_register(&env2).unwrap(),
        DeduplicationOutcome::New
    );
    assert_eq!(
        dedup.check_and_register(&env3).unwrap(),
        DeduplicationOutcome::New
    );
    assert_eq!(dedup.len(), 3);

    // Refresh recency of 1 by duplicate check -> LRU order is now: 2 (oldest), 3, 1 (newest)
    match dedup.check_and_register(&env1).unwrap() {
        DeduplicationOutcome::Duplicate { original_tick, .. } => {
            assert_eq!(original_tick, 1);
        }
        _ => panic!("Expected duplicate for env1"),
    }

    // Insert 4 at capacity 3 -> 2 must be evicted, NOT 1 (even though 1 has smaller CommandId)
    assert_eq!(
        dedup.check_and_register(&env4).unwrap(),
        DeduplicationOutcome::New
    );
    assert_eq!(dedup.len(), 3);

    assert!(
        dedup.get_status(CommandId::from_u128(1)).is_some(),
        "Command 1 was refreshed and must not be evicted"
    );
    assert!(
        dedup.get_status(CommandId::from_u128(2)).is_none(),
        "Command 2 was least recently used and must be evicted"
    );
    assert!(
        dedup.get_status(CommandId::from_u128(3)).is_some(),
        "Command 3 must remain in cache"
    );
    assert!(
        dedup.get_status(CommandId::from_u128(4)).is_some(),
        "Command 4 must remain in cache"
    );
}

/// Verification test: SimulationEngine::submit_command enforces contracts and deduplication.
#[test]
fn test_engine_submit_command_enforces_contracts_and_deduplication() {
    let (mut engine, player) = setup_test_engine(42);

    let cmd1 = IngressCommand::new(
        1001,
        player,
        1,
        1,
        CommandPayload::SetVelocity { dx: 10, dy: 10 },
    );

    // First submission -> Accepted as New
    let res1 = engine.submit_command(cmd1.clone());
    assert_eq!(res1.unwrap(), DeduplicationOutcome::New);

    // Duplicate submission -> Detected as Duplicate
    let res2 = engine.submit_command(cmd1);
    match res2.unwrap() {
        DeduplicationOutcome::Duplicate {
            original_tick,
            status,
        } => {
            assert_eq!(original_tick, 1);
            assert_eq!(status, CommandStatus::Accepted);
        }
        _ => panic!("Expected duplicate detection"),
    }

    // Mismatched payload for same command_id -> IdempotencyConflict error
    let conflict_cmd = IngressCommand::new(
        1001,
        player,
        1,
        2,
        CommandPayload::SetVelocity { dx: 99, dy: 99 },
    );
    let res_conflict = engine.submit_command(conflict_cmd);
    assert!(
        matches!(res_conflict, Err(DomainError::IdempotencyConflict { .. })),
        "Direct submit_command must reject conflicting payload for existing command_id"
    );
}
