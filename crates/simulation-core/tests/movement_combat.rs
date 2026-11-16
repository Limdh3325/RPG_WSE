//! Integration tests for TASK-005: Movement & Combat Minimal Systems.
//!
//! Covers:
//! - TEST-MOV-001: Wall collision and coordinate preservation
//! - TEST-MOV-002: Dynamic obstacle obstruction & diagnostics
//! - TEST-MOV-003: A* search budget exhaustion
//! - TEST-MOV-004: Speed limit enforcement per tick
//! - TEST-CMB-001: Out-of-range attack rejection
//! - TEST-CMB-002: Death produces durable loot intent without direct inventory mutation
//! - TEST-CMB-003: Attack cooldown enforcement
//! - TEST-CMB-004: Monster archetypes (GoblinScout, OrcWarrior) stats & armor mitigation

use ecs_components::{
    components::{CombatStats, Health, MonsterArchetype, Navigation, Position, Velocity},
    region_world::RegionWorld,
};
use game_domain::{
    id::{CommandId, EntityId, RegionId, SessionId},
    state::{CombatStatus, NavigationState},
};
use simulation_core::{
    command::{CommandEnvelope, CommandPayload},
    engine::SimulationEngine,
    event::SimulationEvent,
    pathfinding::{PathError, astar_path},
};

fn create_test_engine(seed: u64) -> SimulationEngine {
    let region_id = RegionId::from_u128(1);
    let world = RegionWorld::new(region_id, 1, 32, 32);
    SimulationEngine::new(world, seed)
}

#[test]
fn test_mov_001_closed_wall_rejection_and_coordinates_unchanged() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();

    // Block tile (3, 1), which corresponds to millitiles 3000..3999 x, 1000..1999 y
    assert!(engine.world.grid.set_tile_blocked(3, 1, true));
    assert!(!engine.world.grid.is_tile_walkable(3, 1));
    assert!(!engine.world.grid.is_millitile_walkable(3500, 1500));

    // Submit Move command to blocked tile
    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 3500,
            target_y: 1500,
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let output = engine.step().unwrap();

    // Verify command was rejected
    let rejected = output.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            command_id: _,
            entity_id,
            reason,
        } => *entity_id == player && reason.contains("not walkable"),
        _ => false,
    });
    assert!(rejected, "Move to blocked wall tile must be rejected");

    // Verify player coordinates remain completely unchanged
    let player_ent = engine.world.get_entity(player).unwrap();
    let pos = engine
        .world
        .world
        .entity(player_ent)
        .get::<Position>()
        .copied()
        .unwrap();
    assert_eq!(pos.x_millitile, 1500, "X coordinate must remain unchanged");
    assert_eq!(pos.y_millitile, 1500, "Y coordinate must remain unchanged");

    // Verify navigation state is Blocked
    let nav = engine
        .world
        .world
        .entity(player_ent)
        .get::<Navigation>()
        .cloned()
        .unwrap();
    assert_eq!(nav.state, NavigationState::Blocked);
}

#[test]
fn test_mov_002_dynamic_obstacle_repath_or_blocked() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();

    // Request move to (1500, 5500)
    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 1500,
            target_y: 5500,
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let out1 = engine.step().unwrap();
    assert!(out1.events.iter().any(|ev| matches!(
        ev,
        SimulationEvent::NavigationStateChanged {
            state: NavigationState::Following,
            ..
        }
    )));

    // Dynamically block the next tile along the path and completely wall off row 3
    engine.world.grid.set_tile_blocked(1, 2, true);
    for x in 0..32 {
        engine.world.grid.set_tile_blocked(x, 3, true);
    }

    let out2 = engine.step().unwrap();
    let blocked_event = out2.events.iter().find(|ev| {
        matches!(
            ev,
            SimulationEvent::NavigationStateChanged {
                state: NavigationState::Blocked,
                ..
            }
        )
    });

    assert!(
        blocked_event.is_some(),
        "Dynamic obstacle must trigger Blocked navigation state"
    );
}

#[test]
fn test_mov_003_astar_budget_exceeded() {
    let region_id = RegionId::from_u128(1);
    let world = RegionWorld::new(region_id, 1, 64, 64);

    let start = (1500, 1500);
    let goal = (60_500, 60_500);

    // Search with tiny budget of 4 expansions
    let result = astar_path(&world.grid, start, goal, 4);

    match result {
        Err(PathError::BudgetExceeded {
            visited_nodes,
            budget,
        }) => {
            assert_eq!(budget, 4);
            assert!(visited_nodes >= 4);
        }
        other => panic!("Expected BudgetExceeded, got {other:?}"),
    }
}

#[test]
fn test_mov_004_speed_limit_enforced() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();

    // Set custom speed to 200 millitiles per tick
    let p_ent = engine.world.get_entity(player).unwrap();
    engine
        .world
        .world
        .entity_mut(p_ent)
        .get_mut::<Navigation>()
        .unwrap()
        .speed_millitiles_per_tick = 200;

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 5000,
            target_y: 1000,
        },
    );
    engine.submit_envelope(cmd).unwrap();

    let out = engine.step().unwrap();

    // Verify distance moved in 1 tick is <= 200 millitiles
    let move_event = out.events.iter().find_map(|ev| match ev {
        SimulationEvent::EntityMoved {
            entity_id,
            from_x,
            from_y,
            to_x,
            to_y,
        } if *entity_id == player => Some((*from_x, *from_y, *to_x, *to_y)),
        _ => None,
    });

    assert!(move_event.is_some(), "EntityMoved event should be emitted");
    let (fx, fy, tx, ty) = move_event.unwrap();
    let dx = tx - fx;
    let dy = ty - fy;
    let dist = ((dx * dx + dy * dy).unsigned_abs().isqrt()) as i64;
    assert!(
        dist <= 200,
        "Distance moved in single tick ({dist}) must not exceed speed (200)"
    );
}

#[test]
fn test_cmb_001_out_of_range_attack_rejected() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let monster = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    engine.spawn_actor(monster, 10_000, 10_000, 100).unwrap();

    // Player range is 1500 millitiles, but monster is 9000 millitiles away
    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: monster },
    );
    engine.submit_envelope(cmd).unwrap();

    let out = engine.step().unwrap();

    let rejected = out.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            command_id: _,
            entity_id,
            reason,
        } => *entity_id == player && reason.contains("out of range"),
        _ => false,
    });
    assert!(
        rejected,
        "Attack outside range must be rejected with 'out of range'"
    );

    // Monster HP remains full
    let m_ent = engine.world.get_entity(monster).unwrap();
    let hp = engine.world.world.entity(m_ent).get::<Health>().unwrap();
    assert_eq!(hp.current, 100, "Target HP must not be damaged");
}

#[test]
fn test_cmb_002_death_produces_durable_intent_no_instant_inventory() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    // Configure player damage to 50
    let p_ent = engine.world.get_entity(player).unwrap();
    engine
        .world
        .world
        .entity_mut(p_ent)
        .get_mut::<CombatStats>()
        .unwrap()
        .base_damage = 50;

    // Spawn GoblinScout at (1500, 1000), within range
    let goblin = engine
        .spawn_monster(MonsterArchetype::GoblinScout, 1500, 1000)
        .unwrap();

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: goblin },
    );
    engine.submit_envelope(cmd).unwrap();

    let out = engine.step().unwrap();

    // Check CombatHit, EntityDied, LootIntentProduced
    let has_hit = out
        .events
        .iter()
        .any(|ev| matches!(ev, SimulationEvent::CombatHit { .. }));
    let has_died = out
        .events
        .iter()
        .any(|ev| matches!(ev, SimulationEvent::EntityDied { entity_id } if *entity_id == goblin));
    let loot_intent = out.events.iter().find_map(|ev| match ev {
        SimulationEvent::LootIntentProduced {
            killer_id,
            victim_id,
            item_id,
            amount,
            intent_id: _,
        } if *killer_id == player && *victim_id == goblin => Some((*item_id, *amount)),
        _ => None,
    });

    assert!(has_hit, "CombatHit must be emitted");
    assert!(has_died, "EntityDied must be emitted");
    assert_eq!(
        loot_intent,
        Some((1001, 1)),
        "GoblinScout default loot intent (item 1001, count 1) must be emitted"
    );

    // Verify durable intent exists in output
    assert_eq!(out.durable_loot_intents.len(), 1);
    assert_eq!(out.durable_loot_intents[0].killer_id, player);
    assert_eq!(out.durable_loot_intents[0].victim_id, goblin);
    assert_eq!(out.durable_loot_intents[0].item_id, 1001);

    // Goblin health is dead
    let g_ent = engine.world.get_entity(goblin).unwrap();
    let g_hp = engine.world.world.entity(g_ent).get::<Health>().unwrap();
    assert_eq!(g_hp.status, CombatStatus::Dead);
    assert_eq!(g_hp.current, 0);
}

#[test]
fn test_cmb_003_attack_cooldown_enforced() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let dummy = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    engine.spawn_actor(dummy, 1200, 1000, 1000).unwrap();

    let p_ent = engine.world.get_entity(player).unwrap();
    let mut p_mut = engine.world.world.entity_mut(p_ent);
    let mut stats = p_mut.get_mut::<CombatStats>().unwrap();
    stats.attack_cooldown_ticks = 3;
    stats.base_damage = 10;

    // Tick 1: First attack -> Succeeds
    let cmd1 = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: dummy },
    );
    engine.submit_envelope(cmd1).unwrap();
    let out1 = engine.step().unwrap();
    assert!(
        out1.events
            .iter()
            .any(|ev| matches!(ev, SimulationEvent::CombatHit { .. }))
    );

    // Tick 2: Second attack immediately -> Rejected due to cooldown
    let cmd2 = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        2,
        2,
        2,
        0,
        CommandPayload::Attack { target_id: dummy },
    );
    engine.submit_envelope(cmd2).unwrap();
    let out2 = engine.step().unwrap();
    let rejected_cooldown = out2.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            entity_id, reason, ..
        } => *entity_id == player && reason.contains("cooldown"),
        _ => false,
    });
    assert!(rejected_cooldown, "Attack during cooldown must be rejected");

    // Tick 3: Advance tick without command
    let _ = engine.step().unwrap();

    // Tick 4: 3 ticks passed (1 -> 4) -> Attack succeeds
    let cmd4 = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        4,
        4,
        4,
        0,
        CommandPayload::Attack { target_id: dummy },
    );
    engine.submit_envelope(cmd4).unwrap();
    let out4 = engine.step().unwrap();
    assert!(
        out4.events
            .iter()
            .any(|ev| matches!(ev, SimulationEvent::CombatHit { .. })),
        "Attack after cooldown must succeed"
    );
}

#[test]
fn test_cmb_004_monster_archetypes_and_combat() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    let p_ent = engine.world.get_entity(player).unwrap();
    engine
        .world
        .world
        .entity_mut(p_ent)
        .get_mut::<CombatStats>()
        .unwrap()
        .base_damage = 20;

    let orc = engine
        .spawn_monster(MonsterArchetype::OrcWarrior, 1500, 1000)
        .unwrap();

    // Orc has 80 HP and 5 Armor. Player base damage is 20. Expected damage: 20 - 5 = 15.
    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: orc },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();

    let hit = out.events.iter().find_map(|ev| match ev {
        SimulationEvent::CombatHit {
            damage,
            remaining_hp,
            ..
        } => Some((*damage, *remaining_hp)),
        _ => None,
    });

    assert_eq!(
        hit,
        Some((15, 65)),
        "Damage must be 15 (20 - 5 armor) and remaining HP 65 (80 - 15)"
    );
}

#[test]
fn test_mov_005_raw_velocity_wall_collision_and_speed_capping() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();
    // Block tile (2, 1) at x: 2000..2999
    engine.world.grid.set_tile_blocked(2, 1, true);

    // 1. Attempt speed tampering: dx=1000 > MAX_SPEED=500
    let cmd_fast = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::SetVelocity { dx: 1000, dy: 0 },
    );
    engine.submit_envelope(cmd_fast).unwrap();
    let out = engine.step().unwrap();
    let rejected_speed = out.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            entity_id, reason, ..
        } => *entity_id == player && reason.contains("speed exceeds maximum authority"),
        _ => false,
    });
    assert!(rejected_speed, "Speed exceeding 500 must be rejected");

    // 2. Set velocity heading directly into wall tile (2, 1)
    let p_ent = engine.world.get_entity(player).unwrap();
    // Remove Navigation to test raw fallback movement
    engine.world.world.entity_mut(p_ent).remove::<Navigation>();
    engine
        .world
        .world
        .entity_mut(p_ent)
        .get_mut::<Velocity>()
        .unwrap()
        .dx = 500;

    // Step tick: player is at 1500 + 500 = 2000 (tile 2, which is blocked!)
    let _out_blocked = engine.step().unwrap();
    let p_pos = engine.world.get_position(player).unwrap();
    assert_eq!(
        p_pos.x_millitile, 1500,
        "Entity must not move into blocked wall tile"
    );
    let p_ent_after = engine.world.get_entity(player).unwrap();
    let vel = engine
        .world
        .world
        .entity(p_ent_after)
        .get::<Velocity>()
        .unwrap();
    assert_eq!(
        vel.dx, 0,
        "Velocity must be stopped upon collision with wall"
    );
}

#[test]
fn test_cmb_005_same_tick_duplicate_attacks_rejected() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let target = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    engine.spawn_actor(target, 1500, 1000, 100).unwrap();

    // Submit TWO attacks from the same player in the exact same tick
    let cmd1 = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: target },
    );
    let cmd2 = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: target },
    );

    engine.submit_envelope(cmd1).unwrap();
    engine.submit_envelope(cmd2).unwrap();

    let out = engine.step().unwrap();
    let hits = out
        .events
        .iter()
        .filter(|ev| matches!(ev, SimulationEvent::CombatHit { .. }))
        .count();
    assert_eq!(hits, 1, "Exactly one attack may connect per tick");

    let duplicate_rejected = out.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            entity_id, reason, ..
        } => *entity_id == player && reason.contains("duplicate attack in tick"),
        _ => false,
    });
    assert!(
        duplicate_rejected,
        "Second attack in same tick must be rejected with cooldown duplicate reason"
    );
}

#[test]
fn test_cmb_006_extreme_coordinate_overflow_safety() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let target = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    // 0. Manhattan distance test with i64::MIN and i64::MAX (extreme signed difference)
    let h = simulation_core::pathfinding::manhattan_distance(
        (i64::MIN, i64::MIN),
        (i64::MAX, i64::MAX),
    );
    assert!(
        h > 0,
        "Extreme Manhattan distance must not overflow or panic"
    );

    // 1. Direct unit test for Position::distance_squared with values that would overflow i64
    let p1 = Position {
        x_millitile: 3_000_000_000,
        y_millitile: 3_000_000_000,
    };
    let p2 = Position {
        x_millitile: -3_000_000_000,
        y_millitile: -3_000_000_000,
    };
    let dist_sq = p1.distance_squared(&p2);
    assert!(
        dist_sq > 0,
        "u128 distance squared must not wrap or overflow"
    );
    assert_eq!(p1.distance_manhattan(&p2), 12_000_000_000);

    // 2. In-bounds distant positions within 32x32 region world
    engine.spawn_actor(player, 0, 0, 100).unwrap();
    engine.spawn_actor(target, 31_000, 31_000, 100).unwrap();

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: target },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();

    let rejected_range = out.events.iter().any(|ev| match ev {
        SimulationEvent::CommandRejected {
            entity_id, reason, ..
        } => *entity_id == player && reason.contains("target out of range"),
        _ => false,
    });
    assert!(
        rejected_range,
        "Extreme coordinate distance must safely reject without integer overflow"
    );
}

#[test]
fn test_cmb_007_loot_intent_deduplication_and_settlement() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1000, 1000, 100).unwrap();
    let p_ent = engine.world.get_entity(player).unwrap();
    engine
        .world
        .world
        .entity_mut(p_ent)
        .get_mut::<CombatStats>()
        .unwrap()
        .base_damage = 50;

    let goblin = engine
        .spawn_monster(MonsterArchetype::GoblinScout, 1500, 1000)
        .unwrap();

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: goblin },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();

    assert_eq!(
        out.durable_loot_intents.len(),
        1,
        "One durable loot intent should be produced on death"
    );
    let intent = &out.durable_loot_intents[0];
    let intent_id = intent.intent_id;

    // Acknowledge and settle the intent
    assert!(!engine.is_loot_intent_settled(&intent_id));
    assert!(engine.acknowledge_loot_intent(intent_id));
    assert!(engine.is_loot_intent_settled(&intent_id));
    assert_eq!(
        engine.pending_loot_intents.len(),
        0,
        "Acknowledged intent must be cleared from pending"
    );
}

#[test]
fn test_mov_006_set_velocity_i32_min_overflow() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();

    let cmd_min_vel = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::SetVelocity {
            dx: i32::MIN,
            dy: i32::MIN,
        },
    );
    engine.submit_envelope(cmd_min_vel).unwrap();
    let out = engine.step().unwrap();
    let rejected = out.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("speed exceeds maximum authority")
    ));
    assert!(
        rejected,
        "SetVelocity with i32::MIN must be safely rejected without panic"
    );
}

#[test]
fn test_mov_007_dynamic_obstacle_repath_around_barrier_to_arrival() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    // Spawn at (1500, 1500) = tile (1, 1)
    engine.spawn_actor(player, 1500, 1500, 200).unwrap();

    // Move to (1500, 4500) = tile (1, 4)
    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 1500,
            target_y: 4500,
        },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();
    assert!(out.events.iter().any(|e| matches!(
        e,
        SimulationEvent::NavigationStateChanged {
            state: NavigationState::Following,
            ..
        }
    )));

    // Block tile (1, 2) dynamically while leaving tile (2, 2) open for detour
    engine.world.grid.set_tile_blocked(1, 2, true);

    // Run ticks until arrived or timeout (max 60 ticks)
    let mut arrived = false;
    for _ in 0..60 {
        let step_out = engine.step().unwrap();
        if step_out.events.iter().any(|e| {
            matches!(
                e,
                SimulationEvent::NavigationStateChanged {
                    state: NavigationState::Arrived,
                    ..
                }
            )
        }) {
            arrived = true;
            break;
        }
    }
    assert!(
        arrived,
        "Actor should successfully replan around dynamic barrier and arrive"
    );

    let p_ent = engine.world.get_entity(player).unwrap();
    let pos = engine.world.world.entity(p_ent).get::<Position>().unwrap();
    assert_eq!(pos.x_millitile, 1500);
    assert_eq!(pos.y_millitile, 4500);
}

#[test]
fn test_mov_008_unreachable_tile_no_path_blocked_diagnostics() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 200).unwrap();

    // Enclose start position (1, 1) completely so open set exhausts before budget
    engine.world.grid.set_tile_blocked(1, 0, true);
    engine.world.grid.set_tile_blocked(1, 2, true);
    engine.world.grid.set_tile_blocked(0, 1, true);
    engine.world.grid.set_tile_blocked(2, 1, true);

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 5500,
            target_y: 5500,
        },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();

    let blocked_event = out.events.iter().find(|e| {
        matches!(
            e,
            SimulationEvent::NavigationStateChanged {
                state: NavigationState::Blocked,
                ..
            }
        )
    });
    assert!(
        blocked_event.is_some(),
        "Unreachable goal must transition to Blocked navigation state"
    );
    if let Some(SimulationEvent::NavigationStateChanged { reason, .. }) = blocked_event {
        assert!(
            reason
                .as_ref()
                .is_some_and(|r| r.contains("no walkable path to goal")),
            "Blocked event should diagnose 'no walkable path to goal'"
        );
    }
}

#[test]
fn test_mov_009_dead_actor_commands_rejected() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();

    // Kill the actor
    let p_ent = engine.world.get_entity(player).unwrap();
    {
        let mut ent_mut = engine.world.world.entity_mut(p_ent);
        let mut hp = ent_mut.get_mut::<Health>().unwrap();
        hp.current = 0;
        hp.status = CombatStatus::Dead;
    }

    // Attempt Move on tick 1
    let cmd_move = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 2500,
            target_y: 2500,
        },
    );
    engine.submit_envelope(cmd_move).unwrap();
    let out_move = engine.step().unwrap();
    assert!(out_move.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("entity is dead")
    )));

    // Attempt SetVelocity on tick 2
    let cmd_vel = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        2,
        2,
        2,
        0,
        CommandPayload::SetVelocity { dx: 100, dy: 100 },
    );
    engine.submit_envelope(cmd_vel).unwrap();
    let out_vel = engine.step().unwrap();
    assert!(out_vel.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("entity is dead")
    )));
}

#[test]
fn test_mov_010_nonexistent_actor_command_rejected() {
    let mut engine = create_test_engine(42);
    let fake_player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        fake_player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Move {
            target_x: 2500,
            target_y: 2500,
        },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();
    assert!(out.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("entity not found")
    )));
}

#[test]
fn test_cmb_008_dead_attacker_or_dead_target_attack_rejected() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();
    let goblin = engine
        .spawn_monster(MonsterArchetype::GoblinScout, 1600, 1500)
        .unwrap();

    // Target already dead
    let g_ent = engine.world.get_entity(goblin).unwrap();
    {
        let mut g_mut = engine.world.world.entity_mut(g_ent);
        let mut g_hp = g_mut.get_mut::<Health>().unwrap();
        g_hp.current = 0;
        g_hp.status = CombatStatus::Dead;
    }

    let cmd_attack_dead = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack { target_id: goblin },
    );
    engine.submit_envelope(cmd_attack_dead).unwrap();
    let out1 = engine.step().unwrap();
    assert!(out1.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("target is not alive")
    )));

    // Revive goblin, kill player
    {
        let mut g_mut2 = engine.world.world.entity_mut(g_ent);
        let mut g_hp2 = g_mut2.get_mut::<Health>().unwrap();
        g_hp2.current = 30;
        g_hp2.status = CombatStatus::Alive;
    }
    let p_ent = engine.world.get_entity(player).unwrap();
    {
        let mut p_mut = engine.world.world.entity_mut(p_ent);
        let mut p_hp = p_mut.get_mut::<Health>().unwrap();
        p_hp.current = 0;
        p_hp.status = CombatStatus::Dead;
    }

    let cmd_dead_player_attack = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        2,
        2,
        2,
        0,
        CommandPayload::Attack { target_id: goblin },
    );
    engine.submit_envelope(cmd_dead_player_attack).unwrap();
    let out2 = engine.step().unwrap();
    assert!(out2.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("entity is dead")
    )));
}

#[test]
fn test_cmb_009_nonexistent_combatants_attack_rejected() {
    let mut engine = create_test_engine(42);
    let player = EntityId::new();
    let session_id = SessionId::new();
    let region_id = engine.world.meta.region_id;

    engine.spawn_actor(player, 1500, 1500, 100).unwrap();
    let fake_target = EntityId::new();

    let cmd = CommandEnvelope::new(
        CommandId::new(),
        player,
        region_id,
        session_id,
        1,
        1,
        1,
        0,
        CommandPayload::Attack {
            target_id: fake_target,
        },
    );
    engine.submit_envelope(cmd).unwrap();
    let out = engine.step().unwrap();
    assert!(out.events.iter().any(|e| matches!(
        e,
        SimulationEvent::CommandRejected { reason, .. } if reason.contains("target entity not found")
    )));
}

#[test]
fn test_cmb_010_deterministic_movement_and_combat_simulation() {
    let mut engine1 = create_test_engine(99);
    let mut engine2 = create_test_engine(99);

    let p1 = EntityId::from_u128(101);
    let p2 = p1; // Same ID for exact replay determinism
    let s_id = SessionId::new();
    let reg = engine1.world.meta.region_id;

    engine1.spawn_actor(p1, 1000, 1000, 200).unwrap();
    engine2.spawn_actor(p2, 1000, 1000, 200).unwrap();

    let g1 = engine1
        .spawn_monster_with_id(
            EntityId::from_u128(202),
            MonsterArchetype::GoblinScout,
            1500,
            1000,
        )
        .unwrap();
    let g2 = engine2
        .spawn_monster_with_id(
            EntityId::from_u128(202),
            MonsterArchetype::GoblinScout,
            1500,
            1000,
        )
        .unwrap();

    // Move and attack sequence
    for t in 1..=30 {
        if t == 1 {
            let cmd_move1 = CommandEnvelope::new(
                CommandId::from_u128(1001),
                p1,
                reg,
                s_id,
                1,
                1,
                t,
                0,
                CommandPayload::Move {
                    target_x: 1400,
                    target_y: 1000,
                },
            );
            let cmd_move2 = CommandEnvelope::new(
                CommandId::from_u128(1001),
                p2,
                reg,
                s_id,
                1,
                1,
                t,
                0,
                CommandPayload::Move {
                    target_x: 1400,
                    target_y: 1000,
                },
            );
            engine1.submit_envelope(cmd_move1).unwrap();
            engine2.submit_envelope(cmd_move2).unwrap();
        } else if t == 5 {
            let cmd_atk1 = CommandEnvelope::new(
                CommandId::from_u128(1002),
                p1,
                reg,
                s_id,
                2,
                5,
                t,
                0,
                CommandPayload::Attack { target_id: g1 },
            );
            let cmd_atk2 = CommandEnvelope::new(
                CommandId::from_u128(1002),
                p2,
                reg,
                s_id,
                2,
                5,
                t,
                0,
                CommandPayload::Attack { target_id: g2 },
            );
            engine1.submit_envelope(cmd_atk1).unwrap();
            engine2.submit_envelope(cmd_atk2).unwrap();
        }

        let out1 = engine1.step().unwrap();
        let out2 = engine2.step().unwrap();

        assert_eq!(out1.tick, out2.tick);
        assert_eq!(
            out1.state_hash, out2.state_hash,
            "State hash must match bit-exact on tick {t}"
        );
        assert_eq!(
            out1.events.len(),
            out2.events.len(),
            "Event counts must match on tick {t}"
        );
    }
}
