//! Tests for Prologue Progression and Idempotent World Transfer (TEST-PRO-001 through TEST-PRO-006).

use simulation_systems::prologue::{
    CharacterPrologueSession, PrologueAction, PrologueManager, PrologueState, PrologueStep,
    TransferStatus,
};
use uuid::Uuid;

#[test]
fn test_pro_001_new_account_full_flow() {
    let mut manager = PrologueManager::new();
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();
    let shared_settlement_id = Uuid::new_v4();
    let mut now = 1000i64;

    // Start prologue
    let initial = manager
        .start_prologue(character_id, private_region_id, now)
        .expect("should start prologue");
    assert!(matches!(
        initial,
        PrologueState::InProgress {
            step: PrologueStep::P0VillageEntry,
            ..
        }
    ));

    // P0: Talk to Elder Elda
    now += 500;
    let p1 = manager
        .advance_step(
            character_id,
            PrologueAction::TalkToNpc {
                npc_name: "Elder Elda".to_string(),
            },
            now,
        )
        .expect("advance to P1");
    assert!(matches!(
        p1,
        PrologueState::InProgress {
            step: PrologueStep::P1ResourceGathering,
            ..
        }
    ));

    // P1: Gather resources (need 3)
    now += 500;
    manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "herb".to_string(),
                amount: 2,
            },
            now,
        )
        .expect("partial gathering");

    now += 500;
    let p2 = manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "timber".to_string(),
                amount: 1,
            },
            now,
        )
        .expect("complete gathering");
    assert!(matches!(
        p2,
        PrologueState::InProgress {
            step: PrologueStep::P2OutpostTravel,
            ..
        }
    ));

    // P2: Reach frontier outpost waypoint
    now += 500;
    let p3 = manager
        .advance_step(
            character_id,
            PrologueAction::ReachWaypoint { x: 120, y: 150 },
            now,
        )
        .expect("reach waypoint");
    assert!(matches!(
        p3,
        PrologueState::InProgress {
            step: PrologueStep::P3RescueCombat,
            ..
        }
    ));

    // P3: Defeat goblin encounter & rescue Kael
    now += 500;
    let p4 = manager
        .advance_step(
            character_id,
            PrologueAction::DefeatEncounter {
                target_id: "goblin_scout_01".to_string(),
            },
            now,
        )
        .expect("defeat encounter");
    assert!(matches!(
        p4,
        PrologueState::InProgress {
            step: PrologueStep::P4SupplyMigration,
            ..
        }
    ));

    // P4: Pack wagon supplies
    now += 500;
    let p5 = manager
        .advance_step(
            character_id,
            PrologueAction::PackSupplies {
                supply_id: "wagon_crate_01".to_string(),
            },
            now,
        )
        .expect("pack supplies");
    assert!(matches!(
        p5,
        PrologueState::InProgress {
            step: PrologueStep::P5SharedSettlement,
            ..
        }
    ));

    // P5: Enter settlement boundary gate -> triggers transfer ready
    now += 500;
    let ready = manager
        .advance_step(character_id, PrologueAction::EnterSettlementGate, now)
        .expect("enter gate");
    assert!(matches!(ready, PrologueState::ReadyToTransfer { .. }));

    // World transfer into shared region
    now += 500;
    let transfer_record = manager
        .transfer_to_shared_world(character_id, shared_settlement_id, now)
        .expect("transfer must succeed");
    assert_eq!(transfer_record.status, TransferStatus::Committed);
    assert_eq!(transfer_record.target_region_id, shared_settlement_id);
    assert!(!transfer_record.rewards.is_empty());

    // Character state is now Completed
    assert!(manager.is_completed(&character_id));
}

#[test]
fn test_pro_002_disconnect_recovery_all_steps() {
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();

    let steps = [
        PrologueStep::P0VillageEntry,
        PrologueStep::P1ResourceGathering,
        PrologueStep::P2OutpostTravel,
        PrologueStep::P3RescueCombat,
        PrologueStep::P4SupplyMigration,
        PrologueStep::P5SharedSettlement,
    ];

    for step in steps {
        let mut manager = PrologueManager::new();
        // Simulate restored checkpoint at step
        let session = CharacterPrologueSession {
            character_id,
            private_region_id,
            state: PrologueState::InProgress {
                step,
                progress: 0,
                target_count: 1,
                fallback_active: false,
                last_failure_reason: None,
                last_updated_ms: 5000,
            },
            rewards_accumulated: Vec::new(),
            fallback_triggered_count: 0,
        };

        manager.restore_session(session).unwrap();

        // Verify recovered state matches exact step
        match manager.get_state(&character_id) {
            PrologueState::InProgress {
                step: recovered_step,
                ..
            } => {
                assert_eq!(recovered_step, step);
            }
            other => panic!("Unexpected recovered state: {:?}", other),
        }
    }
}

#[test]
fn test_pro_003_idempotent_transfer_100_retries() {
    let mut manager = PrologueManager::new();
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();
    let shared_region_id = Uuid::new_v4();
    let now = 1000i64;

    manager
        .start_prologue(character_id, private_region_id, now)
        .unwrap();
    // Fast-track to P5 gate
    manager
        .advance_step(
            character_id,
            PrologueAction::TalkToNpc {
                npc_name: "Elder".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "herbs".to_string(),
                amount: 3,
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::ReachWaypoint { x: 10, y: 10 },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::DefeatEncounter {
                target_id: "goblin".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::PackSupplies {
                supply_id: "box".to_string(),
            },
            now,
        )
        .unwrap();
    let ready_state = manager
        .advance_step(character_id, PrologueAction::EnterSettlementGate, now)
        .unwrap();

    let initial_transfer_id = match ready_state {
        PrologueState::ReadyToTransfer { transfer_id, .. } => transfer_id,
        _ => panic!("Expected ReadyToTransfer"),
    };

    // First execution
    let first_record = manager
        .transfer_to_shared_world(character_id, shared_region_id, now)
        .expect("first transfer must succeed");
    let original_reward_count = first_record.rewards.len();

    // 100 consecutive retries
    for retry_index in 1..=100 {
        let retry_record = manager
            .transfer_to_shared_world(character_id, shared_region_id, now + retry_index)
            .expect("retry must succeed idempotently");

        assert_eq!(
            retry_record.transfer_id, initial_transfer_id,
            "Transfer ID must stay invariant"
        );
        assert_eq!(
            retry_record.rewards.len(),
            original_reward_count,
            "No duplicate item grants allowed"
        );
        assert_eq!(retry_record.status, TransferStatus::Committed);
    }

    // Registry must hold exactly 1 transfer for this character
    assert_eq!(manager.transfer_registry.transfers.len(), 1);
    assert_eq!(
        manager
            .transfer_registry
            .completed_character_transfers
            .get(&character_id),
        Some(&initial_transfer_id)
    );
}

#[test]
fn test_pro_004_completed_character_hides_button() {
    let mut manager = PrologueManager::new();
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();
    let shared_region_id = Uuid::new_v4();
    let now = 1000i64;

    assert!(!manager.is_completed(&character_id));

    // Run to completion
    manager
        .start_prologue(character_id, private_region_id, now)
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::TalkToNpc {
                npc_name: "Elder".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "herbs".to_string(),
                amount: 3,
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::ReachWaypoint { x: 1, y: 1 },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::DefeatEncounter {
                target_id: "goblin".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::PackSupplies {
                supply_id: "box".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(character_id, PrologueAction::EnterSettlementGate, now)
        .unwrap();
    manager
        .transfer_to_shared_world(character_id, shared_region_id, now)
        .unwrap();

    // Verify is_completed is true
    assert!(manager.is_completed(&character_id));

    // Attempting to restart must be rejected
    let restart_err = manager.start_prologue(character_id, private_region_id, now + 100);
    assert_eq!(
        restart_err.err(),
        Some(simulation_systems::prologue::PrologueError::AlreadyCompleted)
    );
}

#[test]
fn test_pro_005_movement_failure_reports_within_5s() {
    let mut manager = PrologueManager::new();
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();
    let now = 1000i64;

    manager
        .start_prologue(character_id, private_region_id, now)
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::TalkToNpc {
                npc_name: "Elder".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "herbs".to_string(),
                amount: 3,
            },
            now,
        )
        .unwrap();

    // Now at P2OutpostTravel
    let failure_time = now + 2000; // 2 seconds later (< 5 seconds)
    let state = manager
        .report_movement_failure(character_id, "BlockedPath: Bridge collapsed", failure_time)
        .expect("report should succeed");

    match state {
        PrologueState::InProgress {
            step,
            fallback_active,
            last_failure_reason,
            last_updated_ms,
            ..
        } => {
            assert_eq!(step, PrologueStep::P2OutpostTravel);
            assert!(fallback_active);
            assert_eq!(
                last_failure_reason,
                Some("BlockedPath: Bridge collapsed".to_string())
            );
            assert!(last_updated_ms - now <= 5000, "Must report within 5000ms");
        }
        other => panic!("Unexpected state: {:?}", other),
    }
}

#[test]
fn test_pro_006_combat_or_npc_death_fallback_path() {
    let mut manager = PrologueManager::new();
    let character_id = Uuid::new_v4();
    let private_region_id = Uuid::new_v4();
    let now = 1000i64;

    manager
        .start_prologue(character_id, private_region_id, now)
        .unwrap();
    // At P0, if Elder Elda is incapacitated or killed
    let fallback_p1 = manager
        .trigger_npc_casualty_fallback(character_id, "Elder Elda", now)
        .expect("fallback must succeed");
    assert!(matches!(
        fallback_p1,
        PrologueState::InProgress {
            step: PrologueStep::P1ResourceGathering,
            fallback_active: true,
            ..
        }
    ));

    // Advance to P3
    manager
        .advance_step(
            character_id,
            PrologueAction::GatherResource {
                resource_type: "salvaged_herbs".to_string(),
                amount: 3,
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(
            character_id,
            PrologueAction::ReachWaypoint { x: 5, y: 5 },
            now,
        )
        .unwrap();

    // At P3, if patrol Kael dies or combat fails
    let fallback_p4 = manager
        .trigger_npc_casualty_fallback(character_id, "Patrol Kael", now)
        .expect("combat fallback must succeed");
    assert!(matches!(
        fallback_p4,
        PrologueState::InProgress {
            step: PrologueStep::P4SupplyMigration,
            fallback_active: true,
            ..
        }
    ));

    // Can still proceed to P5 and complete transfer!
    manager
        .advance_step(
            character_id,
            PrologueAction::PackSupplies {
                supply_id: "wagon".to_string(),
            },
            now,
        )
        .unwrap();
    manager
        .advance_step(character_id, PrologueAction::EnterSettlementGate, now)
        .unwrap();
    let record = manager
        .transfer_to_shared_world(character_id, Uuid::new_v4(), now)
        .unwrap();
    assert_eq!(record.status, TransferStatus::Committed);
    assert!(manager.is_completed(&character_id));
}

#[test]
fn test_pro_007_durability_across_manager_restarts() {
    use simulation_systems::prologue::FilePrologueStorage;
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir().join(format!("prologue_test_{}", Uuid::new_v4()));
    let storage = Arc::new(FilePrologueStorage::new(&temp_dir).unwrap());

    let character_id = Uuid::new_v4();
    let private_region = Uuid::new_v4();
    let target_region = Uuid::new_v4();
    let now = 1_700_000_000_000;

    let committed_transfer_id;
    // 1. Process 1: run prologue through to completion and world transfer
    {
        let mut manager = PrologueManager::new_with_storage(storage.clone());
        manager
            .start_prologue(character_id, private_region, now)
            .unwrap();
        manager
            .advance_step(
                character_id,
                PrologueAction::TalkToNpc {
                    npc_name: "Elda".to_string(),
                },
                now,
            )
            .unwrap();
        manager
            .advance_step(
                character_id,
                PrologueAction::GatherResource {
                    resource_type: "herbs".to_string(),
                    amount: 3,
                },
                now,
            )
            .unwrap();
        manager
            .advance_step(
                character_id,
                PrologueAction::ReachWaypoint { x: 10, y: 10 },
                now,
            )
            .unwrap();
        manager
            .advance_step(
                character_id,
                PrologueAction::DefeatEncounter {
                    target_id: "goblin".to_string(),
                },
                now,
            )
            .unwrap();
        manager
            .advance_step(
                character_id,
                PrologueAction::PackSupplies {
                    supply_id: "wagon".to_string(),
                },
                now,
            )
            .unwrap();
        manager
            .advance_step(character_id, PrologueAction::EnterSettlementGate, now)
            .unwrap();

        let record = manager
            .transfer_to_shared_world(character_id, target_region, now)
            .unwrap();
        assert_eq!(record.status, TransferStatus::Committed);
        committed_transfer_id = record.transfer_id;
        assert!(manager.is_completed(&character_id));
        // manager is dropped here, simulating gateway shutdown/crash
    }

    // 2. Process 2: new gateway instance started from the same durable storage
    {
        let mut recovered_manager = PrologueManager::new_with_storage(storage.clone());

        // Must immediately recognize character as completed (hides button)
        assert!(recovered_manager.is_completed(&character_id));
        assert!(matches!(
            recovered_manager.get_state(&character_id),
            PrologueState::Completed { .. }
        ));

        // Must preserve the exact transfer_id
        assert_eq!(
            recovered_manager
                .transfer_registry
                .completed_character_transfers
                .get(&character_id),
            Some(&committed_transfer_id)
        );

        // Retrying transfer on restarted manager must be 100% idempotent
        let retry_record = recovered_manager
            .transfer_to_shared_world(character_id, target_region, now + 1000)
            .unwrap();
        assert_eq!(retry_record.transfer_id, committed_transfer_id);
        assert_eq!(retry_record.status, TransferStatus::Committed);
        assert_eq!(
            recovered_manager.transfer_registry.issued_receipt_ids.len(),
            retry_record.rewards.len()
        );
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_pro_008_inflight_transfer_crash_reconciliation() {
    use simulation_systems::prologue::{FilePrologueStorage, PrologueStorage};
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir().join(format!("prologue_reconcile_{}", Uuid::new_v4()));
    let storage = Arc::new(FilePrologueStorage::new(&temp_dir).unwrap());

    let character_id = Uuid::new_v4();
    let private_region = Uuid::new_v4();
    let target_region = Uuid::new_v4();
    let transfer_id = Uuid::new_v4();
    let now = 1_700_000_100_000;

    // Simulate an interrupted state: transfer committed in ledger, but session crashed in Transferring
    let session = simulation_systems::prologue::CharacterPrologueSession {
        character_id,
        private_region_id: private_region,
        state: PrologueState::Transferring {
            transfer_id,
            started_at_ms: now,
        },
        rewards_accumulated: Vec::new(),
        fallback_triggered_count: 0,
    };
    storage.save_session(&session).unwrap();

    let committed_record = simulation_systems::prologue::PrologueTransferRecord {
        transfer_id,
        character_id,
        source_region_id: private_region,
        target_region_id: target_region,
        status: TransferStatus::Committed,
        rewards: Vec::new(),
        attempts: 1,
        created_at_ms: now,
        updated_at_ms: now,
    };
    storage.save_transfer(&committed_record).unwrap();

    // Spawn new manager from storage: should automatically reconcile in-flight to Completed
    let recovered_manager =
        simulation_systems::prologue::PrologueManager::new_with_storage(storage.clone());
    assert!(recovered_manager.is_completed(&character_id));
    assert!(matches!(
        recovered_manager.get_state(&character_id),
        PrologueState::Completed { transfer_id: tid, .. } if tid == transfer_id
    ));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_pro_009_uncommitted_inflight_transfer_rollback() {
    use simulation_systems::prologue::{FilePrologueStorage, PrologueStorage};
    use std::sync::Arc;

    let temp_dir =
        std::env::temp_dir().join(format!("prologue_reconcile_rollback_{}", Uuid::new_v4()));
    let storage = Arc::new(FilePrologueStorage::new(&temp_dir).unwrap());

    let character_id = Uuid::new_v4();
    let private_region = Uuid::new_v4();
    let uncommitted_transfer_id = Uuid::new_v4();
    let now = 1_700_000_200_000;

    // Simulate an interrupted state: session crashed in Transferring, but no committed transfer in ledger
    let session = simulation_systems::prologue::CharacterPrologueSession {
        character_id,
        private_region_id: private_region,
        state: PrologueState::Transferring {
            transfer_id: uncommitted_transfer_id,
            started_at_ms: now,
        },
        rewards_accumulated: Vec::new(),
        fallback_triggered_count: 0,
    };
    storage.save_session(&session).unwrap();

    // Spawn new manager from storage: should automatically rollback in-flight to ReadyToTransfer
    let recovered_manager =
        simulation_systems::prologue::PrologueManager::new_with_storage(storage.clone());
    assert!(!recovered_manager.is_completed(&character_id));
    assert!(matches!(
        recovered_manager.get_state(&character_id),
        PrologueState::ReadyToTransfer { transfer_id: tid, .. } if tid == uncommitted_transfer_id
    ));

    let _ = std::fs::remove_dir_all(temp_dir);
}
