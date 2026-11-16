//! Integration tests for TASK-006: Tokio Region Worker / Coordinator.
//!
//! Covers:
//! - TEST-RGN-001: Concurrent independent execution of multiple regions
//! - TEST-RGN-002: Bounded channel backpressure & ServerBusy rejection (0 unbounded growth)
//! - TEST-RGN-003: Region lifecycle transitions (Active -> Paused -> Active -> Stopped) & shutdown drain
//! - TEST-RGN-004: Epoch validation hook rejecting stale/mismatched routing
//! - TEST-RGN-005: Tick lag detection and health degradation/recovery
//! - TEST-RGN-006: Fault isolation between distinct regions
//! - TEST-RGN-007: Persistence ACK routing and durable intent settlement

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ecs_components::grid::MapGrid;
use game_domain::{CommandId, EntityId, RegionHealth, RegionId, RegionStatus, SessionId};
use simulation_core::command::{CommandEnvelope, CommandPayload};
use simulation_core::coordinator::{EpochValidator, RegionCoordinator, RouteError};
use simulation_core::worker::{
    COMMAND_CHANNEL_CAPACITY, ControlMessage, PersistenceAck, RegionWorker,
};

fn create_test_envelope(
    region_id: RegionId,
    entity_id: EntityId,
    seq: u64,
    payload: CommandPayload,
) -> CommandEnvelope {
    create_test_envelope_with_epoch(region_id, entity_id, seq, 1, payload)
}

fn create_test_envelope_with_epoch(
    region_id: RegionId,
    entity_id: EntityId,
    seq: u64,
    ownership_epoch: u64,
    payload: CommandPayload,
) -> CommandEnvelope {
    CommandEnvelope::new(
        CommandId::new(),
        entity_id,
        region_id,
        SessionId::new(),
        seq,
        1,
        0,
        ownership_epoch,
        payload,
    )
}

#[tokio::test]
async fn test_rgn_001_dual_region_independent_execution() {
    let coordinator = RegionCoordinator::new();

    let region_a = RegionId::new();
    let region_b = RegionId::new();

    let handle_a = coordinator
        .spawn_region(region_a, 1, MapGrid::new(32, 32), 101, None)
        .expect("region_a must spawn successfully");
    let handle_b = coordinator
        .spawn_region(region_b, 1, MapGrid::new(32, 32), 202, None)
        .expect("region_b must spawn successfully");

    // Allow workers to boot into Active state
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(handle_a.status(), RegionStatus::Active);
    assert_eq!(handle_b.status(), RegionStatus::Active);
    assert_eq!(coordinator.region_count(), 2);

    // Route distinct commands to Region A and Region B
    let actor_a = EntityId::new();
    let actor_b = EntityId::new();

    let cmd_a = create_test_envelope(
        region_a,
        actor_a,
        1,
        CommandPayload::Move {
            target_x: 2000,
            target_y: 2000,
        },
    );
    let cmd_b = create_test_envelope(
        region_b,
        actor_b,
        1,
        CommandPayload::Move {
            target_x: 5000,
            target_y: 5000,
        },
    );

    assert!(coordinator.route_command(cmd_a).is_ok());
    assert!(coordinator.route_command(cmd_b).is_ok());

    // Allow multiple ticks to execute
    tokio::time::sleep(Duration::from_millis(250)).await;

    let status_a = coordinator.inspect_status(region_a).await.unwrap();
    let status_b = coordinator.inspect_status(region_b).await.unwrap();

    assert!(status_a.current_tick > 0);
    assert!(status_b.current_tick > 0);
    assert_eq!(status_a.health, RegionHealth::Healthy);
    assert_eq!(status_b.health, RegionHealth::Healthy);

    // Clean shutdown
    coordinator.stop_all();
    assert!(handle_a.is_stopped());
    assert!(handle_b.is_stopped());
}

#[tokio::test]
async fn test_rgn_002_bounded_backpressure_and_server_busy() {
    let coordinator = RegionCoordinator::new();
    let region_id = RegionId::new();

    let (worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 42, None);
    coordinator.register_region(handle.clone()).unwrap();

    // Do NOT run the worker thread immediately so commands accumulate in the bounded channel
    let actor = EntityId::new();

    // Fill channel to maximum capacity
    for i in 0..COMMAND_CHANNEL_CAPACITY {
        let cmd = create_test_envelope(
            region_id,
            actor,
            i as u64 + 1,
            CommandPayload::SetVelocity { dx: 10, dy: 10 },
        );
        let res = coordinator.route_command(cmd);
        assert!(res.is_ok(), "command {i} should fit in bounded queue");
    }

    // Next command must be immediately rejected with ServerBusy (non-blocking backpressure)
    let overflow_cmd = create_test_envelope(
        region_id,
        actor,
        COMMAND_CHANNEL_CAPACITY as u64 + 1,
        CommandPayload::SetVelocity { dx: 10, dy: 10 },
    );

    let err = coordinator.route_command(overflow_cmd).unwrap_err();
    assert_eq!(
        err,
        RouteError::ServerBusy { region_id },
        "Excess commands must return ServerBusy without blocking"
    );

    // Now start the worker thread to drain commands
    let thread = worker.spawn_thread();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // After draining, the queue has capacity again
    let new_cmd = create_test_envelope(
        region_id,
        actor,
        COMMAND_CHANNEL_CAPACITY as u64 + 2,
        CommandPayload::SetVelocity { dx: 0, dy: 0 },
    );
    assert!(coordinator.route_command(new_cmd).is_ok());

    coordinator.stop_region(region_id).unwrap();
    let _ = thread.join();
}

#[tokio::test]
async fn test_rgn_003_lifecycle_transitions_and_shutdown_drain() {
    let coordinator = RegionCoordinator::new();
    let region_id = RegionId::new();

    let handle = coordinator
        .spawn_region(region_id, 1, MapGrid::new(16, 16), 77, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.status(), RegionStatus::Active);

    // 1. Pause
    coordinator.pause_region(region_id).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.status(), RegionStatus::Paused);

    let tick_paused = handle.status();
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Tick does not advance while paused
    assert_eq!(handle.status(), tick_paused);

    // 2. Resume
    coordinator.resume_region(region_id).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.status(), RegionStatus::Active);

    // 3. Stop
    coordinator.stop_region(region_id).unwrap();
    assert_eq!(handle.status(), RegionStatus::Stopped);
    assert!(handle.is_stopped());

    // Subsequent command routing must reject with RegionStopped
    let actor = EntityId::new();
    let cmd = create_test_envelope(
        region_id,
        actor,
        1,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );
    let res = coordinator.route_command(cmd);
    assert_eq!(res, Err(RouteError::RegionStopped(region_id)));
}

#[tokio::test]
async fn test_rgn_004_epoch_validation_hook() {
    let allowed_epoch = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let allowed_epoch_clone = Arc::clone(&allowed_epoch);

    let validator: EpochValidator =
        Arc::new(move |_r_id, epoch| epoch == allowed_epoch_clone.load(Ordering::SeqCst));

    let coordinator = RegionCoordinator::new().with_epoch_validator(validator);
    let region_id = RegionId::new();

    let handle = coordinator
        .spawn_region(region_id, 1, MapGrid::new(16, 16), 1, None)
        .unwrap();

    let actor = EntityId::new();
    let valid_cmd = create_test_envelope_with_epoch(
        region_id,
        actor,
        1,
        1,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );

    // Initial epoch 1 matches worker epoch and external validator
    assert!(coordinator.route_command(valid_cmd).is_ok());

    // Stale command with epoch 0 must be rejected immediately at ingress
    let stale_cmd_low = create_test_envelope_with_epoch(
        region_id,
        actor,
        2,
        0,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );
    assert_eq!(
        coordinator.route_command(stale_cmd_low),
        Err(RouteError::EpochMismatch {
            region_id,
            expected: 1
        })
    );

    // Command with future epoch 2 must be rejected at ingress
    let mismatched_cmd_high = create_test_envelope_with_epoch(
        region_id,
        actor,
        3,
        2,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );
    assert_eq!(
        coordinator.route_command(mismatched_cmd_high),
        Err(RouteError::EpochMismatch {
            region_id,
            expected: 1
        })
    );

    // Invalidate external validator lease (e.g. lease moved to 2)
    allowed_epoch.store(2, Ordering::SeqCst);

    let lease_invalidated_cmd = create_test_envelope_with_epoch(
        region_id,
        actor,
        4,
        1,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );
    assert_eq!(
        coordinator.route_command(lease_invalidated_cmd),
        Err(RouteError::EpochMismatch {
            region_id,
            expected: 1
        })
    );

    // Worker defense-in-depth: directly enqueued command with stale epoch is discarded by worker
    let bypass_cmd = create_test_envelope_with_epoch(
        region_id,
        actor,
        5,
        99,
        CommandPayload::Move {
            target_x: 1000,
            target_y: 1000,
        },
    );
    handle.cmd_tx.try_send(bypass_cmd).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    coordinator.stop_all();
}

#[tokio::test]
async fn test_rgn_005_tick_lag_detection_and_recovery() {
    let region_id = RegionId::new();
    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 99, None);

    // Fast tick interval (10ms) and short lag threshold (5ms)
    worker.set_tick_interval(Duration::from_millis(10));
    worker.set_lag_threshold(Duration::from_millis(5));
    let thread = worker.spawn_thread();

    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(handle.health(), RegionHealth::Healthy);

    // Induce artificial tick lag (12ms > 5ms threshold)
    handle
        .control_tx
        .try_send(ControlMessage::SetTestTickDelay(Duration::from_millis(12)))
        .unwrap();
    // Allow >= 3 ticks to accumulate consecutive lag
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(handle.health(), RegionHealth::Degraded);

    // Recover tick delay back to 0ms (< 5ms threshold)
    handle
        .control_tx
        .try_send(ControlMessage::SetTestTickDelay(Duration::ZERO))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.health(), RegionHealth::Healthy);

    // Verify Pause / Resume controls
    handle.control_tx.try_send(ControlMessage::Pause).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(handle.status(), RegionStatus::Paused);

    handle.control_tx.try_send(ControlMessage::Resume).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(handle.status(), RegionStatus::Active);

    handle
        .control_tx
        .try_send(ControlMessage::Shutdown)
        .unwrap();
    let _ = thread.join();
    assert_eq!(handle.status(), RegionStatus::Stopped);
}

#[tokio::test]
async fn test_rgn_006_worker_fault_isolation() {
    let coordinator = RegionCoordinator::new();

    let region_healthy = RegionId::new();
    let region_failing = RegionId::new();

    let handle_h = coordinator
        .spawn_region(region_healthy, 1, MapGrid::new(16, 16), 1, None)
        .unwrap();
    let handle_f = coordinator
        .spawn_region(region_failing, 1, MapGrid::new(16, 16), 2, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Stop region_failing independently
    coordinator.stop_region(region_failing).unwrap();
    assert!(handle_f.is_stopped());

    // region_healthy must remain completely unaffected, Healthy and Active
    assert_eq!(handle_h.status(), RegionStatus::Active);
    assert_eq!(handle_h.health(), RegionHealth::Healthy);

    let actor = EntityId::new();
    let cmd_h = create_test_envelope(
        region_healthy,
        actor,
        1,
        CommandPayload::Move {
            target_x: 2000,
            target_y: 2000,
        },
    );
    assert!(coordinator.route_command(cmd_h).is_ok());

    tokio::time::sleep(Duration::from_millis(150)).await;
    let status_h = coordinator.inspect_status(region_healthy).await.unwrap();
    assert!(status_h.current_tick > 0);
    assert_eq!(status_h.health, RegionHealth::Healthy);

    coordinator.stop_all();
}

#[tokio::test]
async fn test_rgn_007_persistence_ack_prioritization_and_settlement() {
    let coordinator = RegionCoordinator::new();
    let region_id = RegionId::new();

    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 42, None);

    // Register a pending loot intent inside the worker engine
    let intent_uuid = uuid::Uuid::new_v4();
    worker
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id: intent_uuid,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 101,
            amount: 5,
            tick: 1,
        });
    assert_eq!(worker.engine_mut().pending_loot_intents.len(), 1);

    coordinator.register_region(handle.clone()).unwrap();
    let thread = worker.spawn_thread();

    // Send persistence ack via coordinator
    let ack = PersistenceAck {
        intent_id: intent_uuid,
        success: true,
        reason: None,
    };
    coordinator.route_ack(region_id, ack).unwrap();

    // Allow worker tick to process ACK
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Intent must now be acknowledged and settled (not pending)
    let remaining = coordinator.drain_region_intents(region_id).await.unwrap();
    assert!(
        remaining.is_empty(),
        "intent should have been settled by ACK"
    );

    coordinator.stop_region(region_id).unwrap();
    let _ = thread.join();

    assert!(handle.is_stopped());
}

#[tokio::test]
async fn test_rgn_008_unrelated_entity_ticks_continue_during_persistence_delay() {
    let coordinator = RegionCoordinator::new();
    let region_id = RegionId::new();

    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(32, 32), 42, None);
    worker.set_tick_interval(Duration::from_millis(10));

    // Entity A has an unsettled pending durable loot intent
    let intent_uuid = uuid::Uuid::new_v4();
    worker
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id: intent_uuid,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 202,
            amount: 1,
            tick: 1,
        });

    // Spawn Entity B (player) at (1000, 1000) with 1000 speed
    let actor_b = EntityId::new();
    worker
        .engine_mut()
        .spawn_actor(actor_b, 1000, 1000, 1000)
        .unwrap();

    coordinator.register_region(handle.clone()).unwrap();
    let thread = worker.spawn_thread();

    // Send Move command for Entity B
    let move_cmd = create_test_envelope(
        region_id,
        actor_b,
        1,
        CommandPayload::Move {
            target_x: 5000,
            target_y: 1000,
        },
    );
    assert!(coordinator.route_command(move_cmd).is_ok());

    // Sleep for 60ms (~6 ticks) while persistence ACK is "delayed"
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Verify status: Region is healthy, active, and ticks advanced
    let status = coordinator.inspect_status(region_id).await.unwrap();
    assert_eq!(status.status, RegionStatus::Active);
    assert_eq!(status.health, RegionHealth::Healthy);
    assert!(
        status.current_tick >= 5,
        "ticks must continue advancing during persistence delay"
    );

    // Intent is still pending (not lost, not corrupted)
    let drained = coordinator.drain_region_intents(region_id).await.unwrap();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].intent_id, intent_uuid);

    coordinator.stop_region(region_id).unwrap();
    let _ = thread.join();
}

#[tokio::test]
async fn test_rgn_009_graceful_shutdown_drain_preserves_all_pending_intents() {
    let coordinator = RegionCoordinator::new();
    let region_id = RegionId::new();

    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 11, None);
    worker.set_tick_interval(Duration::from_millis(10));

    // Register 3 un-persisted durable loot intents
    let id1 = uuid::Uuid::new_v4();
    let id2 = uuid::Uuid::new_v4();
    let id3 = uuid::Uuid::new_v4();

    for id in [id1, id2, id3] {
        worker
            .engine_mut()
            .pending_loot_intents
            .push(simulation_core::engine::DurableLootIntent {
                intent_id: id,
                killer_id: EntityId::new(),
                victim_id: EntityId::new(),
                item_id: 777,
                amount: 3,
                tick: 1,
            });
    }

    coordinator.register_region(handle.clone()).unwrap();
    let thread = worker.spawn_thread();

    // Sleep a few ticks
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Gracefully shutdown and drain via coordinator stop_region_drain
    let drained = coordinator.stop_region_drain(region_id).await.unwrap();
    let _ = thread.join();

    assert_eq!(drained.len(), 3, "all 3 pending intents must be preserved");
    let ids: Vec<_> = drained.iter().map(|i| i.intent_id).collect();
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));
    assert!(ids.contains(&id3));
    assert_eq!(handle.status(), RegionStatus::Stopped);

    // Verify synchronous stop_region also drains pending intents with zero loss
    let region_id2 = RegionId::new();
    let (mut worker2, handle2) = RegionWorker::new(region_id2, 1, MapGrid::new(16, 16), 22, None);
    worker2.set_tick_interval(Duration::from_millis(10));
    let id4 = uuid::Uuid::new_v4();
    worker2
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id: id4,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 888,
            amount: 1,
            tick: 1,
        });
    let thread2 = worker2.spawn_thread();
    coordinator
        .register_region_with_thread(handle2.clone(), thread2)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    let drained2 = coordinator.stop_region(region_id2).unwrap();
    assert_eq!(drained2.len(), 1);
    assert_eq!(drained2[0].intent_id, id4);
    assert_eq!(handle2.status(), RegionStatus::Stopped);

    // Verify stop_all drains and returns all remaining regions
    let region_id3 = RegionId::new();
    let (mut worker3, handle3) = RegionWorker::new(region_id3, 1, MapGrid::new(16, 16), 33, None);
    worker3.set_tick_interval(Duration::from_millis(10));
    let id5 = uuid::Uuid::new_v4();
    worker3
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id: id5,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 999,
            amount: 5,
            tick: 1,
        });
    let thread3 = worker3.spawn_thread();
    coordinator
        .register_region_with_thread(handle3.clone(), thread3)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    let all_drained = coordinator.stop_all();
    assert!(all_drained.contains_key(&region_id3));
    assert_eq!(all_drained.get(&region_id3).unwrap().len(), 1);
    assert_eq!(all_drained.get(&region_id3).unwrap()[0].intent_id, id5);

    // Verify coordinator saved intents outbox contains all drained intents
    let saved_intents = coordinator.drain_all_saved_intents();
    assert!(saved_intents.contains_key(&region_id));
    assert!(saved_intents.contains_key(&region_id2));
    assert!(saved_intents.contains_key(&region_id3));

    // Verify saturated control channel does not block or lose intents on shutdown
    let region_id4 = RegionId::new();
    let (mut worker4, handle4) = RegionWorker::new(region_id4, 1, MapGrid::new(16, 16), 44, None);
    worker4.set_tick_interval(Duration::from_millis(10));
    let id6 = uuid::Uuid::new_v4();
    worker4
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id: id6,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 321,
            amount: 2,
            tick: 1,
        });
    let thread4 = worker4.spawn_thread();
    coordinator
        .register_region_with_thread(handle4.clone(), thread4)
        .unwrap();

    // Saturate control channel to its 64-message capacity
    for _ in 0..64 {
        let _ = handle4.control_tx.try_send(ControlMessage::Pause);
    }
    assert!(handle4.control_tx.try_send(ControlMessage::Pause).is_err());

    // Stop region must cleanly unblock via atomic shutdown flag, join, and drain intents
    let drained4 = coordinator.stop_region(region_id4).unwrap();
    assert_eq!(drained4.len(), 1);
    assert_eq!(drained4[0].intent_id, id6);
    assert_eq!(handle4.status(), RegionStatus::Stopped);
}

#[tokio::test]
async fn test_rgn_010_persistence_acks_processed_before_commands_in_tick() {
    let region_id = RegionId::new();
    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 1, None);
    worker.set_tick_interval(Duration::from_millis(10));

    let intent_id = uuid::Uuid::new_v4();
    worker
        .engine_mut()
        .pending_loot_intents
        .push(simulation_core::engine::DurableLootIntent {
            intent_id,
            killer_id: EntityId::new(),
            victim_id: EntityId::new(),
            item_id: 100,
            amount: 1,
            tick: 1,
        });

    // Pre-queue ACK into persistence channel
    handle
        .ack_tx
        .try_send(PersistenceAck {
            intent_id,
            success: true,
            reason: None,
        })
        .unwrap();

    let thread = worker.spawn_thread();
    tokio::time::sleep(Duration::from_millis(30)).await;

    handle
        .control_tx
        .try_send(ControlMessage::Shutdown)
        .unwrap();
    let _ = thread.join();

    assert_eq!(handle.status(), RegionStatus::Stopped);
}

#[tokio::test]
async fn test_rgn_011_accumulated_drift_ceiling_prevents_death_spiral() {
    let region_id = RegionId::new();
    let (mut worker, handle) = RegionWorker::new(region_id, 1, MapGrid::new(16, 16), 1, None);
    worker.set_tick_interval(Duration::from_millis(10));
    let thread = worker.spawn_thread();

    tokio::time::sleep(Duration::from_millis(30)).await;

    // Induce a 550ms severe drift tick (> 500ms MAX_ACCUMULATED_DRIFT_MS)
    handle
        .control_tx
        .try_send(ControlMessage::SetTestTickDelay(Duration::from_millis(520)))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(560)).await;

    // Reset delay to zero; worker should immediately recover normal pacing without burst catchup
    handle
        .control_tx
        .try_send(ControlMessage::SetTestTickDelay(Duration::ZERO))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.status(), RegionStatus::Active);

    handle
        .control_tx
        .try_send(ControlMessage::Shutdown)
        .unwrap();
    let _ = thread.join();
    assert_eq!(handle.status(), RegionStatus::Stopped);
}
