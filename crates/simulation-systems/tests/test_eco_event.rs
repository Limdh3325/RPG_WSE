//! Verification test suite for TASK-011: Economy, Faction, and Dynamic Event Rules.
//!
//! Covers:
//! - TEST-ECO-001: Production < Consumption -> Stock depletion, bounded price hike
//! - TEST-ECO-002: Concurrent purchase contention -> Exactly one succeeds, no negative stock
//! - TEST-ECO-003: Food ledger audit -> Double-entry conservation, negative stock rejection
//! - TEST-EVENT-001: Threshold edge transition -> Single shortage event, duplicate suppression
//! - TEST-EVENT-002: Dynamic rule AST validator -> Rejection of duplicate rules and invalid AST
//! - TEST-EVENT-003: Shortage -> Caravan -> Ambush chain autonomous completion without input
//! - TEST-EVENT-004: Escort vs Raid differential outcomes, faction reputations, and news logging

use game_domain::{DomainError, EntityId};
use simulation_core::SimulationEvent;
use simulation_systems::economy::{FoodLedger, FoodLedgerOp, SettlementEconomy};
use simulation_systems::event_chain::{CaravanEventChain, PlayerCaravanIntervention};
use simulation_systems::faction::{FactionId, FactionManager};
use simulation_systems::rule_dsl::{
    ActionAST, ComparisonOp, ConditionAST, DynamicRule, DynamicRuleRegistry, MetricKey,
};

#[test]
fn test_eco_001_production_less_than_consumption_causes_price_hike_bounded() {
    let mut economy = SettlementEconomy::new_default(1);
    let mut ledger = FoodLedger::new();

    // Configure deficit condition: active_farms produce 10, but 10 population consumes 20
    economy.active_farms = 1;
    economy.base_yield_per_farm = 10;
    economy.population = 10;
    economy.food_consumption_per_person = 2; // consumption = 20
    economy.food_stock = 100;
    economy.target_stock = 150;
    economy.base_price = 10;
    economy.min_price = 5;
    economy.max_price = 50;
    economy.current_price = 10;
    economy.price_ema_factor = 50;

    let initial_stock = economy.food_stock;
    let initial_price = economy.current_price;

    // Simulate 5 steps of daily economy
    for tick in 1..=5 {
        economy.step_settlement_economy(tick, 100, &mut ledger);
    }

    // Invariant: Stock should decrease because production (10) < consumption (20)
    assert!(
        economy.food_stock < initial_stock,
        "food stock must decrease under net deficit"
    );
    // Invariant: Price should rise due to scarcity
    assert!(
        economy.current_price > initial_price,
        "price must increase as scarcity rises"
    );
    // Invariant: Price must not exceed max_price
    assert!(
        economy.current_price <= economy.max_price,
        "price must respect max_price bound"
    );
}

#[test]
fn test_eco_002_concurrent_purchase_contention_and_negative_stock_rejection() {
    let mut economy = SettlementEconomy::new_default(2);
    let mut ledger = FoodLedger::new();

    // Only 1 food unit remaining
    economy.food_stock = 1;
    economy.current_price = 15;

    let buyer_a = EntityId::from_u128(1001);
    let buyer_b = EntityId::from_u128(1002);

    // First purchase of 1 unit
    let res_a = economy.execute_purchase(1, buyer_a, 1, &mut ledger);
    assert!(
        res_a.is_ok(),
        "First buyer must successfully purchase remaining stock"
    );
    assert_eq!(economy.food_stock, 0, "Stock must drop to exactly 0");

    // Second purchase of 1 unit must fail due to OutOfStock
    let res_b = economy.execute_purchase(2, buyer_b, 1, &mut ledger);
    assert!(
        matches!(res_b, Err(DomainError::OutOfStock { .. })),
        "Second buyer must be rejected with OutOfStock error"
    );

    // Invariant: Food stock must NEVER be negative
    assert_eq!(economy.food_stock, 0, "Food stock must never drop below 0");
}

#[test]
fn test_eco_003_food_ledger_strict_conservation_and_invariants() {
    let mut ledger = FoodLedger::new();
    let settlement_id = 3;
    let mut current_stock = 50;

    // Record production (+30)
    current_stock = ledger
        .record(
            1,
            settlement_id,
            FoodLedgerOp::Production { amount: 30 },
            current_stock,
            30,
        )
        .expect("production record should succeed");
    assert_eq!(current_stock, 80);

    // Record consumption (-20)
    current_stock = ledger
        .record(
            2,
            settlement_id,
            FoodLedgerOp::Consumption { amount: 20 },
            current_stock,
            -20,
        )
        .expect("consumption record should succeed");
    assert_eq!(current_stock, 60);

    // Record purchase (-10)
    current_stock = ledger
        .record(
            3,
            settlement_id,
            FoodLedgerOp::Purchase {
                amount: 10,
                buyer_id: EntityId::from_u128(99),
                unit_price: 12,
            },
            current_stock,
            -10,
        )
        .expect("purchase record should succeed");
    assert_eq!(current_stock, 50);

    // Verify conservation audit
    assert!(
        ledger.audit_conservation(),
        "ledger audit must confirm mathematical conservation of balance"
    );

    // Attempt invalid operation that violates non-negative invariant (-100 when stock is 50)
    let invalid_res = ledger.record(
        4,
        settlement_id,
        FoodLedgerOp::Consumption { amount: 100 },
        current_stock,
        -100,
    );
    assert!(
        matches!(invalid_res, Err(DomainError::NegativeFoodStock { .. })),
        "ledger must strictly reject operations causing negative stock"
    );
}

#[test]
fn test_event_001_threshold_edge_transition_and_duplicate_suppression() {
    let mut economy = SettlementEconomy::new_default(4);
    let mut ledger = FoodLedger::new();

    economy.food_stock = 60;
    economy.shortage_threshold = 50;
    economy.shortage_active = false;
    economy.active_farms = 0;
    economy.population = 5;
    economy.food_consumption_per_person = 4; // consumption = 20 per step

    // Step 1: stock drops from 60 -> 40 (< 50). Edge transition false -> true!
    let events_step_1 = economy.step_settlement_economy(1, 100, &mut ledger);
    assert_eq!(economy.food_stock, 40);
    assert!(economy.shortage_active);
    assert_eq!(
        events_step_1.len(),
        1,
        "exactly one FoodShortage event must be emitted on threshold crossing"
    );
    assert!(matches!(
        events_step_1[0],
        SimulationEvent::FoodShortage { deficit: 10, .. }
    ));

    // Step 2: stock drops from 40 -> 20 (< 50). Condition still true, but edge was already crossed!
    let events_step_2 = economy.step_settlement_economy(2, 100, &mut ledger);
    assert_eq!(economy.food_stock, 20);
    assert!(economy.shortage_active);
    assert!(
        events_step_2.is_empty(),
        "no duplicate FoodShortage events should be emitted during persistent shortage"
    );

    // Step 3: Resupply caravan arrives (+80 food)
    economy
        .apply_caravan_resupply(3, "Caravan-4", 80, &mut ledger)
        .expect("resupply should succeed");
    assert_eq!(economy.food_stock, 100);
    assert!(
        !economy.shortage_active,
        "shortage_active must be cleared once stock exceeds threshold"
    );
}

#[test]
fn test_event_002_rule_ast_validation_and_duplicate_rule_rejection() {
    let mut registry = DynamicRuleRegistry::new();

    let rule_a = DynamicRule {
        id: "RULE_FOOD_SHORTAGE".to_string(),
        name: "Food Shortage Caravan Trigger".to_string(),
        condition: ConditionAST::EdgeTransition {
            metric: MetricKey::FoodStock(10),
            op: ComparisonOp::Lt,
            value: 50,
        },
        actions: vec![ActionAST::DispatchCaravan {
            caravan_id: "CARAVAN_10".to_string(),
            settlement_id: 10,
            food_amount: 100,
        }],
        cooldown_ticks: 30,
        last_triggered_tick: None,
    };

    // First registration must succeed
    assert!(
        registry.register_rule(rule_a.clone()).is_ok(),
        "first rule registration must succeed"
    );
    assert_eq!(registry.rule_count(), 1);

    // Second registration with identical ID must fail with DuplicateRule (DoD requirement!)
    let dup_res = registry.register_rule(rule_a);
    assert!(
        matches!(dup_res, Err(DomainError::DuplicateRule { .. })),
        "duplicate rule registration must be strictly rejected"
    );

    // Empty actions must be rejected by validator
    let invalid_rule = DynamicRule {
        id: "INVALID_EMPTY_RULE".to_string(),
        name: "Invalid Empty Rule".to_string(),
        condition: ConditionAST::MetricThreshold {
            metric: MetricKey::SecurityIndex(10),
            op: ComparisonOp::Gt,
            value: 0,
        },
        actions: vec![],
        cooldown_ticks: 0,
        last_triggered_tick: None,
    };
    assert!(
        matches!(
            registry.register_rule(invalid_rule),
            Err(DomainError::InvalidRuleAst { .. })
        ),
        "rule with empty actions must fail validation"
    );
}

#[test]
fn test_event_003_shortage_caravan_ambush_chain_autonomous_completion_without_input() {
    let mut economy = SettlementEconomy::new_default(5);
    let mut faction_mgr = FactionManager::new();
    let mut ledger = FoodLedger::new();

    economy.food_stock = 10;
    economy.security_index = 70; // Moderately secure

    let mut caravan_chain =
        CaravanEventChain::new_shortage_request("Caravan-Auto".to_string(), 5, 1, 150);

    assert!(caravan_chain.is_active());

    // Run ticks without ANY player input (player ignores the event)
    let mut tick = 1;
    let mut dispatched_seen = false;
    let mut ambushed_seen = false;
    let mut resolved_seen = false;

    while caravan_chain.is_active() && tick <= 30 {
        let events = caravan_chain.step(
            tick,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            None, // No player actor
            None, // No player intervention (player is idle/spectating)
        );

        for event in events {
            match event {
                SimulationEvent::CaravanDispatched { .. } => dispatched_seen = true,
                SimulationEvent::CaravanAmbushed { .. } => ambushed_seen = true,
                SimulationEvent::CaravanResolved {
                    outcome,
                    food_delivered,
                    ..
                } => {
                    resolved_seen = true;
                    assert!(
                        outcome == "escort_succeeded" || outcome == "raid_succeeded",
                        "outcome must be conclusive"
                    );
                    if outcome == "escort_succeeded" {
                        assert_eq!(food_delivered, 150);
                    }
                }
                _ => {}
            }
        }
        tick += 1;
    }

    // Invariant: Caravan must autonomously complete the whole chain without getting stuck!
    assert!(dispatched_seen, "Caravan must have dispatched autonomously");
    assert!(
        ambushed_seen,
        "Caravan must have triggered ambush during journey"
    );
    assert!(
        resolved_seen,
        "Caravan must have resolved autonomously via timeout/NPC combat"
    );
    assert!(
        !caravan_chain.is_active(),
        "Caravan chain must not be stuck in active state"
    );
    assert!(
        !caravan_chain.news_log.is_empty(),
        "News log must have captured announcements during the journey"
    );
}

#[test]
fn test_event_004_escort_vs_raid_differential_outcomes_and_news_logging() {
    let player = EntityId::from_u128(9999);

    // Case A: Player assists escort -> EscortSucceeded
    {
        let mut economy = SettlementEconomy::new_default(6);
        let mut faction_mgr = FactionManager::new();
        let mut ledger = FoodLedger::new();
        economy.food_stock = 20;
        economy.security_index = 50;

        let mut chain =
            CaravanEventChain::new_shortage_request("Caravan-Escort".to_string(), 6, 1, 100);

        // Tick 1: Dispatch
        chain.step(
            1,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            None,
        );
        // Tick 4: Ambush triggers mid-journey
        chain.step(
            4,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            None,
        );
        // Tick 5: Player supports escort
        let events = chain.step(
            5,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            Some(PlayerCaravanIntervention::EscortSupport {
                combat_contribution: 50,
            }),
        );

        assert!(
            events.iter().any(|e| matches!(e, SimulationEvent::CaravanResolved { outcome, .. } if outcome == "escort_succeeded")),
            "Caravan should resolve as escort_succeeded with player escort help"
        );
        assert_eq!(
            economy.food_stock, 120,
            "Food stock should increase by resupply amount"
        );
        assert!(economy.security_index > 50, "Security index should improve");
        assert!(faction_mgr.get_reputation(player, FactionId::Settlers) > 0);
        assert!(
            chain
                .news_log
                .iter()
                .any(|msg| msg.contains("식량을 보급했습니다"))
        );
    }

    // Case B: Player joins bandit raid -> RaidSucceeded
    {
        let mut economy = SettlementEconomy::new_default(7);
        let mut faction_mgr = FactionManager::new();
        let mut ledger = FoodLedger::new();
        economy.food_stock = 20;
        economy.security_index = 50;

        let mut chain =
            CaravanEventChain::new_shortage_request("Caravan-Raid".to_string(), 7, 1, 100);

        // Tick 1: Dispatch
        chain.step(
            1,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            None,
        );
        // Tick 4: Ambush triggers
        chain.step(
            4,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            None,
        );
        // Tick 5: Player joins raid
        let events = chain.step(
            5,
            &mut economy,
            &mut faction_mgr,
            &mut ledger,
            Some(player),
            Some(PlayerCaravanIntervention::JoinRaid {
                raid_contribution: 60,
            }),
        );

        assert!(
            events.iter().any(|e| matches!(e, SimulationEvent::CaravanResolved { outcome, .. } if outcome == "raid_succeeded")),
            "Caravan should resolve as raid_succeeded when player aids bandits"
        );
        assert_eq!(
            economy.food_stock, 20,
            "Food stock should NOT increase on raid"
        );
        assert!(
            economy.security_index < 50,
            "Security index should drop on raid"
        );
        assert!(faction_mgr.get_reputation(player, FactionId::Bandits) > 0);
        assert!(faction_mgr.get_reputation(player, FactionId::Settlers) < 0);
        assert!(
            chain
                .news_log
                .iter()
                .any(|msg| msg.contains("약탈당했습니다"))
        );
    }
}
