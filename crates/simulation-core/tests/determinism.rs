use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use ecs_components::region_world::RegionWorld;
use game_domain::{
    id::{EntityId, RegionId},
    state::{CombatStatus, EntityStateDto, HealthDto, PersonalityDto, PositionDto, VelocityDto},
};
use simulation_core::{
    command::{CommandPayload, IngressCommand},
    engine::SimulationEngine,
};

fn create_test_region(region_id: RegionId, entity_count: usize) -> RegionWorld {
    let mut world = RegionWorld::new_default_64x64(region_id, 1);
    for i in 0..entity_count {
        let id = EntityId::from_u128((i + 1) as u128);
        let x = ((i as i64 * 300) % 60_000) + 1_000;
        let y = (((i * 7) as i64 * 300) % 60_000) + 1_000;

        let entity_dto = EntityStateDto {
            id,
            position: PositionDto {
                x_millitile: x,
                y_millitile: y,
            },
            velocity: Some(VelocityDto { dx: 10, dy: -10 }),
            health: Some(HealthDto {
                current: 100,
                max: 100,
                status: CombatStatus::Alive,
            }),
            needs: None,
            personality: if i % 2 == 0 {
                Some(PersonalityDto {
                    courage: 50,
                    greed: 30,
                    loyalty: 70,
                })
            } else {
                None
            },
            inventory: None,
            faction: None,
            collider: None,
        };

        world.spawn_entity(entity_dto).expect("spawn entity");
    }
    world
}

/// TEST-TICK-001: 10,000 headless ticks with identical seed and inputs produce
/// identical final state hash and identical execution summary.
#[test]
fn test_tick_001_ten_thousand_ticks_determinism() {
    let region_id = RegionId::from_u128(100);
    let seed = 987654321_u64;
    let ticks = 10_000_u64;

    // Run 1
    let world1 = create_test_region(region_id, 20);
    let mut engine1 = SimulationEngine::new(world1, seed);

    let start = Instant::now();
    let summary1 = engine1.run_ticks(ticks).expect("run ticks 1");
    let duration1 = start.elapsed();

    // Run 2
    let world2 = create_test_region(region_id, 20);
    let mut engine2 = SimulationEngine::new(world2, seed);
    let summary2 = engine2.run_ticks(ticks).expect("run ticks 2");

    assert_eq!(
        summary1.total_ticks_executed, ticks,
        "must execute exactly 10,000 ticks"
    );
    assert_eq!(
        summary1.final_state_hash, summary2.final_state_hash,
        "final state hash must match identically across independent runs"
    );
    assert_eq!(
        summary1.total_events_emitted, summary2.total_events_emitted,
        "event counts must match identically"
    );
    assert_eq!(
        summary1.npc_decision_counts, summary2.npc_decision_counts,
        "npc decision counts must match identically"
    );

    println!(
        "TEST-TICK-001: 10,000 ticks completed in {:?}, hash={}",
        duration1, summary1.final_state_hash
    );
}

/// TEST-TICK-002: NPC scheduled tick execution counts are exact.
#[test]
fn test_tick_002_npc_scheduled_decision_counts_exact() {
    let region_id = RegionId::new();
    let seed = 42_u64;
    let total_ticks = 1_000_u64;

    let world = create_test_region(region_id, 10);
    let mut engine = SimulationEngine::new(world, seed);
    engine.npc_decision_interval = 5;

    let summary = engine.run_ticks(total_ticks).expect("run ticks");

    // In 1,000 ticks with interval 5, exactly 1000 / 5 = 200 decisions must be executed for each active NPC
    assert!(
        !summary.npc_decision_counts.is_empty(),
        "must have registered NPCs"
    );
    for (npc_id, count) in &summary.npc_decision_counts {
        assert_eq!(
            *count, 200,
            "NPC {npc_id} must execute exactly 200 scheduled decisions in 1,000 ticks with interval 5"
        );
    }
}

/// TEST-TICK-003: Shuffled / out-of-order ingress commands produce identical sorted execution
/// and identical final state hash.
#[test]
fn test_tick_003_shuffled_ingress_commands_deterministic_ordering() {
    let region_id = RegionId::new();
    let seed = 777_u64;

    let entity_id = EntityId::new();
    let mut world1 = RegionWorld::new_default_64x64(region_id, 1);
    let entity_dto = EntityStateDto {
        id: entity_id,
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
    world1.spawn_entity(entity_dto.clone()).unwrap();

    let mut world2 = RegionWorld::new_default_64x64(region_id, 1);
    world2.spawn_entity(entity_dto).unwrap();

    let mut engine1 = SimulationEngine::new(world1, seed);
    let mut engine2 = SimulationEngine::new(world2, seed);

    let cmd_a = IngressCommand::new(
        1,
        entity_id,
        1,
        1,
        CommandPayload::Move {
            target_x: 15_000,
            target_y: 10_000,
        },
    );
    let cmd_b = IngressCommand::new(
        2,
        entity_id,
        1,
        2,
        CommandPayload::Move {
            target_x: 20_000,
            target_y: 20_000,
        },
    );
    let cmd_c = IngressCommand::new(
        3,
        entity_id,
        1,
        3,
        CommandPayload::SetVelocity { dx: 50, dy: 50 },
    );

    // Engine 1 receives in order A -> B -> C
    engine1
        .submit_command(cmd_a.clone())
        .expect("valid command A");
    engine1
        .submit_command(cmd_b.clone())
        .expect("valid command B");
    engine1
        .submit_command(cmd_c.clone())
        .expect("valid command C");

    // Engine 2 receives in reverse/shuffled order C -> A -> B
    engine2.submit_command(cmd_c).expect("valid command C");
    engine2.submit_command(cmd_a).expect("valid command A");
    engine2.submit_command(cmd_b).expect("valid command B");

    let out1 = engine1.step().expect("step 1");
    let out2 = engine2.step().expect("step 2");

    assert_eq!(
        out1.state_hash, out2.state_hash,
        "shuffled commands must produce identical state hash due to deterministic admission sorting"
    );
    assert_eq!(
        out1.events, out2.events,
        "shuffled commands must produce identical event stream"
    );
}

/// TEST-TICK-004: Overrun simulation: tick numbers are sequential and never skipped even when overruns occur.
#[test]
fn test_tick_004_overrun_never_skips_ticks() {
    let region_id = RegionId::new();
    let seed = 123_u64;

    let world = create_test_region(region_id, 5);
    let mut engine = SimulationEngine::new(world, seed);

    let mut observed_ticks = Vec::new();

    for i in 1..=20 {
        let elapsed = if i % 3 == 0 {
            std::time::Duration::from_millis(150)
        } else {
            std::time::Duration::from_millis(20)
        };
        let out = engine.step_with_telemetry(elapsed).expect("step");
        observed_ticks.push(out.tick);
    }

    let expected_ticks: Vec<u64> = (1..=20).collect();
    assert_eq!(
        observed_ticks, expected_ticks,
        "ticks must be strictly sequential 1..=20 without any skipped numbers despite overruns"
    );
    assert!(
        engine.clock.overrun_count > 0,
        "overruns must be recorded in clock metrics"
    );
}

/// TEST-TICK-004B: Engine telemetry connects execution elapsed time to clock overrun,
/// degradation (>=5 consecutive overruns), and on-time recovery without state variance.
#[test]
fn test_tick_004_engine_telemetry_overrun_degradation_and_recovery() {
    use simulation_core::clock::ClockStatus;

    let region_id = RegionId::new();
    let seed = 456_u64;
    let world = create_test_region(region_id, 3);
    let mut engine = SimulationEngine::new(world, seed);

    assert_eq!(engine.clock.status, ClockStatus::Normal);

    // 4 consecutive overruns (150ms > 100ms): should still be Normal
    for _ in 0..4 {
        engine
            .step_with_telemetry(std::time::Duration::from_millis(150))
            .expect("step");
        assert_eq!(engine.clock.status, ClockStatus::Normal);
    }

    // 5th consecutive overrun: transitions to Degraded
    engine
        .step_with_telemetry(std::time::Duration::from_millis(150))
        .expect("step");
    assert_eq!(
        engine.clock.status,
        ClockStatus::Degraded,
        "5 consecutive overruns must transition engine clock status to Degraded"
    );

    // On-time tick (30ms <= 100ms): transitions back to Normal
    engine
        .step_with_telemetry(std::time::Duration::from_millis(30))
        .expect("step");
    assert_eq!(
        engine.clock.status,
        ClockStatus::Normal,
        "on-time tick execution must recover clock status back to Normal"
    );

    // Standard engine.step() measures real execution and keeps status valid
    let real_step = engine.step().expect("step");
    assert_eq!(real_step.tick, 7);
    assert_eq!(engine.clock.status, ClockStatus::Normal);
}

/// TEST-TICK-005: Asynchronous external delay does not block or increase logical tick simulation time.
#[test]
fn test_tick_005_async_external_delay_does_not_block_simulation() {
    let region_id = RegionId::new();
    let seed = 999_u64;

    let world = create_test_region(region_id, 5);
    let mut engine = SimulationEngine::new(world, seed);

    let background_work_done = Arc::new(AtomicBool::new(false));
    let flag_clone = Arc::clone(&background_work_done);

    // Spawn an asynchronous background worker simulating heavy IO or LLM delay
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        flag_clone.store(true, Ordering::SeqCst);
    });

    let start = Instant::now();
    // Execute 50 ticks immediately
    for _ in 0..50 {
        let _ = engine.step().expect("step");
    }
    let elapsed = start.elapsed();

    // 50 logical ticks represent 50 * 100ms = 5000ms logical time, but must complete in <100ms wall-clock time
    assert!(
        elapsed < Duration::from_millis(150),
        "headless simulation of 50 ticks must complete rapidly without awaiting external delay; took {:?}",
        elapsed
    );
    assert_eq!(engine.clock.current_tick, 50);
}
