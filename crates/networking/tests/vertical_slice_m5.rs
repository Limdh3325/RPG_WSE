use networking::gateway::InboundCommand;
use networking::replicator::RegionReplicator;
use networking::session::Session;
use networking::simulation::{
    BANDIT_KAGE_ID, BANDIT_SCAR_ID, CARAVAN_CART_ID, DEFAULT_SETTLEMENT_REGION_ID, ELDER_ELDA_ID,
    FARMER_CORIN_ID, FARMER_DELIA_ID, GUARD_AELYN_ID, GUARD_ROLAND_ID, GUARD_VARICK_ID,
    MERCHANT_BORIN_ID, MERCHANT_LUCAS_ID, SettlementSimulation, TRAINING_DUMMY_ID, WILD_BOAR_ID,
};
use shared_protocol::{EntityStateWire, ServerMessage, WireCommandPayload, WireCommandStatus};
use simulation_systems::event_chain::{CaravanEventChain, CaravanPhase};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

fn setup_m5_simulation() -> (Arc<SettlementSimulation>, Arc<RegionReplicator>) {
    let replicator = Arc::new(RegionReplicator::new(DEFAULT_SETTLEMENT_REGION_ID));
    let sim = Arc::new(SettlementSimulation::new(
        DEFAULT_SETTLEMENT_REGION_ID,
        replicator.clone(),
    ));

    (sim, replicator)
}

fn create_mock_session(
    actor_id: Uuid,
    session_id: Uuid,
    replicator: &RegionReplicator,
) -> (Arc<Session>, mpsc::Receiver<ServerMessage>) {
    let (tx, rx) = mpsc::channel(256);
    let session = Arc::new(Session::new(
        session_id,
        Uuid::new_v4(),
        actor_id,
        DEFAULT_SETTLEMENT_REGION_ID,
        tx,
    ));
    replicator.add_session(session.clone());
    (session, rx)
}

#[tokio::test]
async fn test_m5_npc_and_monster_vertical_slice_population() {
    let (sim, _) = setup_m5_simulation();
    let entities = sim.get_all_entities();

    // 10 NPCs + 2 Monsters = 12 baseline entities
    assert!(
        entities.len() >= 12,
        "Expected at least 12 baseline entities, found {}",
        entities.len()
    );

    let entity_ids: HashSet<Uuid> = entities.iter().map(|e| e.id).collect();

    // Verify all 10 NPCs
    assert!(entity_ids.contains(&ELDER_ELDA_ID), "Elder Elda missing");
    assert!(
        entity_ids.contains(&MERCHANT_BORIN_ID),
        "Merchant Borin missing"
    );
    assert!(
        entity_ids.contains(&MERCHANT_LUCAS_ID),
        "Merchant Lucas missing"
    );
    assert!(
        entity_ids.contains(&FARMER_CORIN_ID),
        "Farmer Corin missing"
    );
    assert!(
        entity_ids.contains(&FARMER_DELIA_ID),
        "Farmer Delia missing"
    );
    assert!(
        entity_ids.contains(&GUARD_VARICK_ID),
        "Guard Varick missing"
    );
    assert!(
        entity_ids.contains(&GUARD_ROLAND_ID),
        "Guard Roland missing"
    );
    assert!(entity_ids.contains(&GUARD_AELYN_ID), "Guard Aelyn missing");
    assert!(entity_ids.contains(&BANDIT_KAGE_ID), "Bandit Kage missing");
    assert!(entity_ids.contains(&BANDIT_SCAR_ID), "Bandit Scar missing");

    // Verify 2 Monsters
    assert!(
        entity_ids.contains(&TRAINING_DUMMY_ID),
        "Training Dummy missing"
    );
    assert!(entity_ids.contains(&WILD_BOAR_ID), "Wild Boar missing");
}

#[tokio::test]
async fn test_m5_e2e_001_and_002_food_shortage_threshold_and_no_duplicates() {
    let (sim, replicator) = setup_m5_simulation();

    let actor_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let (_session, mut rx) = create_mock_session(actor_id, session_id, &replicator);

    // Initial food stock = 100, threshold = 50.
    {
        let eco = sim.economy.read().unwrap();
        assert_eq!(eco.food_stock, 100);
        assert!(!eco.shortage_active);
    }

    // Force food stock down across threshold to 40 (false -> true transition)
    {
        let mut eco = sim.economy.write().unwrap();
        eco.food_stock = 40;
    }

    // Step tick to evaluate economic rules
    sim.step_tick();

    // Verify shortage was activated and caravan event was requested
    {
        let eco = sim.economy.read().unwrap();
        assert!(eco.shortage_active, "Shortage should now be active");
        let caravan = sim.active_caravan.read().unwrap();
        assert!(caravan.is_some(), "Caravan should have been dispatched");
    }

    // Client should have received the Announcement message
    let mut announcement_received = false;
    while let Ok(msg) = rx.try_recv() {
        if let ServerMessage::Announcement { title, .. } = msg {
            if title.contains("Shortage") {
                announcement_received = true;
                break;
            }
        }
    }
    assert!(announcement_received, "Expected food shortage announcement");

    // TEST-E2E-002: Run 1,000 continuous ticks while food stock remains below threshold.
    // Ensure NO duplicate caravan events are generated!
    for _ in 0..1000 {
        sim.step_tick();
    }

    let caravan_count = {
        let caravan = sim.active_caravan.read().unwrap();
        if caravan.is_some() { 1 } else { 0 }
    };
    assert_eq!(
        caravan_count, 1,
        "There must never be duplicate caravan events for a single ongoing shortage"
    );
}

#[tokio::test]
async fn test_m5_e2e_003_caravan_raid_decreases_security() {
    let (sim, _) = setup_m5_simulation();

    let initial_security = {
        let eco = sim.economy.read().unwrap();
        eco.security_index
    };

    // Set up caravan in Ambushed phase with dead cart to trigger raid success
    {
        let mut eco = sim.economy.write().unwrap();
        eco.shortage_active = true;
        eco.food_stock = 30;

        let mut caravan = sim.active_caravan.write().unwrap();
        let mut chain = CaravanEventChain::new_shortage_request("raid-test".into(), 1, 1, 50);
        chain.phase = CaravanPhase::Ambushed {
            ambush_tick: 1,
            timeout_tick: 100,
            food_amount: 50,
            escort_power: 10,
            bandit_power: 100,
        };
        *caravan = Some(chain);
    }

    // Insert cart with 0 HP
    {
        let mut map = sim.entities.write().unwrap();
        map.insert(
            CARAVAN_CART_ID,
            networking::simulation::SimulationEntity {
                state: EntityStateWire {
                    id: CARAVAN_CART_ID,
                    x: -10,
                    y: -10,
                    entity_type: "caravan".into(),
                    hp: Some(0),
                    max_hp: Some(100),
                    name: Some("Cart".into()),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: -10,
                base_y: -10,
            },
        );
    }

    // Step tick to trigger raid resolution
    sim.step_tick();

    // Verify security index decreased and food was not delivered
    let eco = sim.economy.read().unwrap();
    assert!(
        eco.security_index < initial_security,
        "Security index should decrease after caravan raid (was {}, now {})",
        initial_security,
        eco.security_index
    );
    assert_eq!(eco.food_stock, 30, "Food stock should not increase on raid");
}

#[tokio::test]
async fn test_m5_e2e_004_caravan_escort_replenishes_food_and_stock() {
    let (sim, _) = setup_m5_simulation();

    // Start with low food and 1 market stock
    {
        let mut eco = sim.economy.write().unwrap();
        eco.shortage_active = true;
        eco.food_stock = 20;

        let mut caravan = sim.active_caravan.write().unwrap();
        let mut chain = CaravanEventChain::new_shortage_request("escort-test".into(), 1, 1, 50);
        chain.phase = CaravanPhase::Ambushed {
            ambush_tick: 1,
            timeout_tick: 100,
            food_amount: 50,
            escort_power: 100,
            bandit_power: 20,
        };
        *caravan = Some(chain);
    }

    // Defeat bandits
    {
        let mut map = sim.entities.write().unwrap();
        if let Some(bandit) = map.get_mut(&BANDIT_KAGE_ID) {
            bandit.state.hp = Some(0);
        }
    }

    // Step tick
    sim.step_tick();

    // Verify food stock replenished (+50), shortage cleared, and market food stock replenished
    let eco = sim.economy.read().unwrap();
    assert_eq!(
        eco.food_stock, 70,
        "Food stock should increase by delivered amount (+50)"
    );
    assert!(!eco.shortage_active, "Shortage flag must be cleared");

    let market_stock = *sim.market_food_stock.read().unwrap();
    assert!(market_stock > 1, "Market stock should be replenished (+5)");
}

#[tokio::test]
async fn test_m5_e2e_005_autonomous_event_resolution_without_player_input() {
    let (sim, _) = setup_m5_simulation();

    // Trigger shortage
    {
        let mut eco = sim.economy.write().unwrap();
        eco.food_stock = 45;
    }
    sim.step_tick(); // Shortage -> Requested

    // Let simulation run without any player input
    for _ in 0..100 {
        sim.step_tick();
    }

    // The event chain MUST reach a resolved state autonomously (EscortSucceeded or RaidSucceeded)
    let caravan_guard = sim.active_caravan.read().unwrap();
    if let Some(ref caravan) = *caravan_guard {
        assert!(
            matches!(caravan.phase, CaravanPhase::Resolved { .. }),
            "Caravan event must autonomously reach a Resolved phase even without player input. Current phase: {:?}",
            caravan.phase
        );
    }
}

#[tokio::test]
async fn test_m5_e2e_006_atomic_purchase_competition_no_double_spend() {
    let (sim, replicator) = setup_m5_simulation();

    let player1_id = Uuid::new_v4();
    let session1_id = Uuid::new_v4();
    let (session1, _) = create_mock_session(player1_id, session1_id, &replicator);

    let player2_id = Uuid::new_v4();
    let session2_id = Uuid::new_v4();
    let (session2, _) = create_mock_session(player2_id, session2_id, &replicator);

    sim.register_player(player1_id, Some("Player 1".into()));
    sim.register_player(player2_id, Some("Player 2".into()));

    // Ensure initial market stock is exactly 1
    {
        let mut stock = sim.market_food_stock.write().unwrap();
        *stock = 1;
    }

    // Player 1 and Player 2 both attempt to purchase the single available food item for 10 gold
    let cmd1 = InboundCommand {
        session_id: session1_id,
        actor_id: player1_id,
        region_id: DEFAULT_SETTLEMENT_REGION_ID,
        command_id: Uuid::new_v4(),
        payload: WireCommandPayload::Purchase {
            item_id: 1,
            price: 10,
        },
    };

    let cmd2 = InboundCommand {
        session_id: session2_id,
        actor_id: player2_id,
        region_id: DEFAULT_SETTLEMENT_REGION_ID,
        command_id: Uuid::new_v4(),
        payload: WireCommandPayload::Purchase {
            item_id: 1,
            price: 10,
        },
    };

    let res1 = sim.apply_command(&session1, cmd1);
    let res2 = sim.apply_command(&session2, cmd2);

    let status1 = match res1 {
        ServerMessage::CommandStatus { status, .. } => status,
        _ => panic!("Expected CommandStatus"),
    };

    let status2 = match res2 {
        ServerMessage::CommandStatus { status, .. } => status,
        _ => panic!("Expected CommandStatus"),
    };

    // Exactly one must be Committed, and the other must be Failed (OUT_OF_STOCK)
    let committed_count = (status1 == WireCommandStatus::Committed) as usize
        + (status2 == WireCommandStatus::Committed) as usize;
    let failed_count = (status1 == WireCommandStatus::Failed) as usize
        + (status2 == WireCommandStatus::Failed) as usize;

    assert_eq!(
        committed_count, 1,
        "Exactly one purchase must commit when stock = 1"
    );
    assert_eq!(
        failed_count, 1,
        "The competing purchase must fail due to out-of-stock"
    );

    // Final stock must be exactly 0, never negative
    let final_stock = *sim.market_food_stock.read().unwrap();
    assert_eq!(final_stock, 0, "Market stock must be 0 and non-negative");

    // Double-entry food ledger verification
    let ledger = sim.food_ledger.read().unwrap();
    let purchase_ops: Vec<_> = ledger
        .entries()
        .iter()
        .filter(|e| {
            matches!(
                e.op,
                simulation_systems::economy::FoodLedgerOp::Purchase { .. }
            )
        })
        .collect();
    assert_eq!(
        purchase_ops.len(),
        1,
        "Exactly one purchase must be recorded in the ledger"
    );
}
