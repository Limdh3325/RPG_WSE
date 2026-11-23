use crate::gateway::InboundCommand;
use crate::replicator::RegionReplicator;
use crate::session::SharedSession;
use shared_protocol::{EntityStateWire, ServerMessage, WireCommandPayload, WireCommandStatus};
use simulation_systems::economy::{FoodLedger, FoodLedgerOp, SettlementEconomy};
use simulation_systems::event_chain::{CaravanEventChain, CaravanPhase, CaravanResolution};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

pub const DEFAULT_SETTLEMENT_REGION_ID: Uuid = Uuid::from_u128(1);

// 10 Settlement NPCs (Vertical Slice)
pub const ELDER_ELDA_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000101);
pub const MERCHANT_BORIN_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000102);
pub const MERCHANT_LUCAS_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000105);
pub const FARMER_CORIN_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000106);
pub const FARMER_DELIA_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000107);
pub const GUARD_VARICK_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000108);
pub const GUARD_ROLAND_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000109);
pub const GUARD_AELYN_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_00000000010a);
pub const BANDIT_KAGE_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_00000000010b);
pub const BANDIT_SCAR_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_00000000010c);

// 2 Monsters (Vertical Slice)
pub const TRAINING_DUMMY_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000103);
pub const WILD_BOAR_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000104);

// Event Entity
pub const CARAVAN_CART_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000201);

#[derive(Debug, Clone)]
pub struct SimulationEntity {
    pub state: EntityStateWire,
    pub respawn_at_tick: Option<u64>,
    pub wander_dir: i32,
    pub base_x: i32,
    pub base_y: i32,
}

pub struct SettlementSimulation {
    pub region_id: Uuid,
    pub entities: Arc<RwLock<HashMap<Uuid, SimulationEntity>>>,
    /// Persisted player state across disconnects: (x, y, hp, max_hp)
    pub persistent_player_states: Arc<RwLock<HashMap<Uuid, (i32, i32, i32, i32)>>>,
    pub tick: AtomicU64,
    pub replicator: Arc<RegionReplicator>,

    // M5 Economy & Dynamic Events
    pub economy: Arc<RwLock<SettlementEconomy>>,
    pub food_ledger: Arc<RwLock<FoodLedger>>,
    pub market_food_stock: Arc<RwLock<i64>>,
    pub player_gold: Arc<RwLock<HashMap<Uuid, i64>>>,
    pub active_caravan: Arc<RwLock<Option<CaravanEventChain>>>,
}

impl SettlementSimulation {
    pub fn new(region_id: Uuid, replicator: Arc<RegionReplicator>) -> Self {
        let mut map = HashMap::new();

        // 1. Elder Elda (Town Elder)
        map.insert(
            ELDER_ELDA_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: ELDER_ELDA_ID,
                    x: 5,
                    y: -5,
                    entity_type: "npc".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Elder Elda".into()),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: 5,
                base_y: -5,
            },
        );

        // 2. Merchant Borin (General Shopkeeper)
        map.insert(
            MERCHANT_BORIN_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: MERCHANT_BORIN_ID,
                    x: -8,
                    y: -5,
                    entity_type: "merchant".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Merchant Borin".into()),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: -8,
                base_y: -5,
            },
        );

        // 3. Merchant Lucas (Trade Merchant)
        map.insert(
            MERCHANT_LUCAS_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: MERCHANT_LUCAS_ID,
                    x: -12,
                    y: -5,
                    entity_type: "merchant".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Merchant Lucas".into()),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: -12,
                base_y: -5,
            },
        );

        // 4. Farmer Corin
        map.insert(
            FARMER_CORIN_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: FARMER_CORIN_ID,
                    x: 15,
                    y: -10,
                    entity_type: "npc".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Farmer Corin".into()),
                },
                respawn_at_tick: None,
                wander_dir: 1,
                base_x: 15,
                base_y: -10,
            },
        );

        // 5. Farmer Delia
        map.insert(
            FARMER_DELIA_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: FARMER_DELIA_ID,
                    x: 18,
                    y: -8,
                    entity_type: "npc".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Farmer Delia".into()),
                },
                respawn_at_tick: None,
                wander_dir: -1,
                base_x: 18,
                base_y: -8,
            },
        );

        // 6. Guard Captain Varick
        map.insert(
            GUARD_VARICK_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: GUARD_VARICK_ID,
                    x: 0,
                    y: 10,
                    entity_type: "guard".into(),
                    hp: Some(150),
                    max_hp: Some(150),
                    name: Some("Guard Varick".into()),
                },
                respawn_at_tick: None,
                wander_dir: 1,
                base_x: 0,
                base_y: 10,
            },
        );

        // 7. Guard Roland
        map.insert(
            GUARD_ROLAND_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: GUARD_ROLAND_ID,
                    x: -15,
                    y: 5,
                    entity_type: "guard".into(),
                    hp: Some(120),
                    max_hp: Some(120),
                    name: Some("Guard Roland".into()),
                },
                respawn_at_tick: None,
                wander_dir: 1,
                base_x: -15,
                base_y: 5,
            },
        );

        // 8. Guard Aelyn
        map.insert(
            GUARD_AELYN_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: GUARD_AELYN_ID,
                    x: 15,
                    y: 5,
                    entity_type: "guard".into(),
                    hp: Some(120),
                    max_hp: Some(120),
                    name: Some("Guard Aelyn".into()),
                },
                respawn_at_tick: None,
                wander_dir: -1,
                base_x: 15,
                base_y: 5,
            },
        );

        // 9. Bandit Kage
        map.insert(
            BANDIT_KAGE_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: BANDIT_KAGE_ID,
                    x: 25,
                    y: 25,
                    entity_type: "bandit".into(),
                    hp: Some(90),
                    max_hp: Some(90),
                    name: Some("Bandit Kage".into()),
                },
                respawn_at_tick: None,
                wander_dir: 1,
                base_x: 25,
                base_y: 25,
            },
        );

        // 10. Bandit Scar
        map.insert(
            BANDIT_SCAR_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: BANDIT_SCAR_ID,
                    x: 28,
                    y: 22,
                    entity_type: "bandit".into(),
                    hp: Some(90),
                    max_hp: Some(90),
                    name: Some("Bandit Scar".into()),
                },
                respawn_at_tick: None,
                wander_dir: -1,
                base_x: 28,
                base_y: 22,
            },
        );

        // 11. Training Dummy (Monster 1)
        map.insert(
            TRAINING_DUMMY_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: TRAINING_DUMMY_ID,
                    x: 12,
                    y: 8,
                    entity_type: "monster".into(),
                    hp: Some(100),
                    max_hp: Some(100),
                    name: Some("Training Dummy".into()),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: 12,
                base_y: 8,
            },
        );

        // 12. Wild Boar (Monster 2)
        map.insert(
            WILD_BOAR_ID,
            SimulationEntity {
                state: EntityStateWire {
                    id: WILD_BOAR_ID,
                    x: -10,
                    y: 12,
                    entity_type: "monster".into(),
                    hp: Some(80),
                    max_hp: Some(80),
                    name: Some("Wild Boar".into()),
                },
                respawn_at_tick: None,
                wander_dir: 1,
                base_x: -10,
                base_y: 12,
            },
        );

        // Initial Economy: 10 population, 100 food, 50 shortage threshold, 10 gold base price
        let economy = SettlementEconomy::new(1, 10, 100, 500, 200, 10, 50);
        let mut ledger = FoodLedger::new();
        // Record initial inventory
        let _ = ledger.record(1, 1, FoodLedgerOp::Production { amount: 100 }, 0, 100);

        Self {
            region_id,
            entities: Arc::new(RwLock::new(map)),
            persistent_player_states: Arc::new(RwLock::new(HashMap::new())),
            tick: AtomicU64::new(1),
            replicator,
            economy: Arc::new(RwLock::new(economy)),
            food_ledger: Arc::new(RwLock::new(ledger)),
            market_food_stock: Arc::new(RwLock::new(1)), // Initially 1 food stock for competition test
            player_gold: Arc::new(RwLock::new(HashMap::new())),
            active_caravan: Arc::new(RwLock::new(None)),
        }
    }

    pub fn current_tick(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }

    pub fn get_player_gold(&self, actor_id: &Uuid) -> i64 {
        let gold_map = self.player_gold.read().unwrap_or_else(|p| p.into_inner());
        *gold_map.get(actor_id).unwrap_or(&20) // Default 20 gold per player
    }

    pub fn set_player_gold(&self, actor_id: Uuid, gold: i64) {
        let mut gold_map = self.player_gold.write().unwrap_or_else(|p| p.into_inner());
        gold_map.insert(actor_id, gold);
    }

    pub fn register_player(&self, actor_id: Uuid, name: Option<String>) -> EntityStateWire {
        // Ensure player wallet initialized
        {
            let mut gold_map = self.player_gold.write().unwrap_or_else(|p| p.into_inner());
            gold_map.entry(actor_id).or_insert(20);
        }

        let (saved_x, saved_y, saved_hp, saved_max_hp) = {
            let p_states = self
                .persistent_player_states
                .read()
                .unwrap_or_else(|p| p.into_inner());
            p_states.get(&actor_id).copied().unwrap_or((0, 0, 100, 100))
        };

        let mut map = self
            .entities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entity = map.entry(actor_id).or_insert_with(|| {
            let hero_name = name.unwrap_or_else(|| {
                let id_str = actor_id.to_string();
                format!("Hero {}", &id_str[..4])
            });
            info!(
                %actor_id,
                %hero_name,
                saved_x,
                saved_y,
                "Spawning player into settlement simulation"
            );
            SimulationEntity {
                state: EntityStateWire {
                    id: actor_id,
                    x: saved_x,
                    y: saved_y,
                    entity_type: "player".into(),
                    hp: Some(saved_hp),
                    max_hp: Some(saved_max_hp),
                    name: Some(hero_name),
                },
                respawn_at_tick: None,
                wander_dir: 0,
                base_x: saved_x,
                base_y: saved_y,
            }
        });
        entity.state.clone()
    }

    pub fn unregister_player(&self, actor_id: &Uuid) {
        let mut map = self
            .entities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(removed) = map.remove(actor_id) {
            // Keep persistent record for reconnect
            let mut p_states = self
                .persistent_player_states
                .write()
                .unwrap_or_else(|p| p.into_inner());
            p_states.insert(
                *actor_id,
                (
                    removed.state.x,
                    removed.state.y,
                    removed.state.hp.unwrap_or(100),
                    removed.state.max_hp.unwrap_or(100),
                ),
            );
            info!(%actor_id, "Despawned player entity from live simulation (state preserved)");
        }
    }

    pub fn get_all_entities(&self) -> Vec<EntityStateWire> {
        let map = self
            .entities
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.values().map(|e| e.state.clone()).collect()
    }

    pub fn apply_command(&self, _session: &SharedSession, cmd: InboundCommand) -> ServerMessage {
        let current_tick = self.current_tick();

        match cmd.payload {
            WireCommandPayload::Move { target_x, target_y } => {
                let mut map = self
                    .entities
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(entity) = map.get_mut(&cmd.actor_id) {
                    entity.state.x = target_x as i32;
                    entity.state.y = target_y as i32;

                    // Update persistent state
                    let mut p_states = self
                        .persistent_player_states
                        .write()
                        .unwrap_or_else(|p| p.into_inner());
                    p_states.insert(
                        cmd.actor_id,
                        (
                            entity.state.x,
                            entity.state.y,
                            entity.state.hp.unwrap_or(100),
                            entity.state.max_hp.unwrap_or(100),
                        ),
                    );

                    debug!(
                        actor_id = %cmd.actor_id,
                        x = entity.state.x,
                        y = entity.state.y,
                        "Player moved"
                    );
                    ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Committed,
                        reason: None,
                        committed_event_seq: Some(current_tick),
                    }
                } else {
                    ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Rejected,
                        reason: Some("Entity not found in simulation".into()),
                        committed_event_seq: None,
                    }
                }
            }
            WireCommandPayload::Attack { target_id } => {
                let mut map = self
                    .entities
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(target) = map.get_mut(&target_id) {
                    let current_hp = target.state.hp.unwrap_or(100);
                    let new_hp = (current_hp - 25).max(0);
                    target.state.hp = Some(new_hp);
                    info!(
                        actor_id = %cmd.actor_id,
                        %target_id,
                        new_hp,
                        "Target attacked"
                    );

                    if new_hp == 0 {
                        // Schedule respawn after 30 ticks (~3 seconds)
                        target.respawn_at_tick = Some(current_tick + 30);
                        info!(%target_id, "Target slain, respawn scheduled");
                    }

                    ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Committed,
                        reason: None,
                        committed_event_seq: Some(current_tick),
                    }
                } else {
                    ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Rejected,
                        reason: Some("Target entity not found".into()),
                        committed_event_seq: None,
                    }
                }
            }
            WireCommandPayload::Purchase { item_id, price } => {
                // Strict atomic purchase evaluation (TEST-E2E-006):
                // Fixed lock order: market_stock -> player_gold -> food_ledger
                let mut stock_guard = self
                    .market_food_stock
                    .write()
                    .unwrap_or_else(|p| p.into_inner());
                let mut gold_guard = self.player_gold.write().unwrap_or_else(|p| p.into_inner());

                let player_gold = gold_guard.entry(cmd.actor_id).or_insert(20);

                if *stock_guard <= 0 {
                    info!(actor_id = %cmd.actor_id, item_id, "Purchase failed: Out of stock");
                    return ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Failed,
                        reason: Some("OUT_OF_STOCK".into()),
                        committed_event_seq: None,
                    };
                }

                if *player_gold < price {
                    info!(actor_id = %cmd.actor_id, current_gold = *player_gold, price, "Purchase rejected: Insufficient funds");
                    return ServerMessage::CommandStatus {
                        command_id: cmd.command_id,
                        status: WireCommandStatus::Rejected,
                        reason: Some("INSUFFICIENT_FUNDS".into()),
                        committed_event_seq: None,
                    };
                }

                // Deduct stock and gold
                *stock_guard -= 1;
                *player_gold -= price;

                // Record into double-entry food ledger
                {
                    let mut ledger = self.food_ledger.write().unwrap_or_else(|p| p.into_inner());
                    let mut eco = self.economy.write().unwrap_or_else(|p| p.into_inner());
                    let prev_balance = eco.food_stock;
                    let _ = ledger.record(
                        current_tick,
                        eco.settlement_id,
                        FoodLedgerOp::Purchase {
                            amount: 1,
                            buyer_id: game_domain::EntityId(cmd.actor_id),
                            unit_price: price,
                        },
                        prev_balance,
                        -1,
                    );
                    eco.food_stock = (eco.food_stock - 1).max(0);
                }

                info!(
                    actor_id = %cmd.actor_id,
                    item_id,
                    price,
                    remaining_stock = *stock_guard,
                    remaining_gold = *player_gold,
                    "Purchase committed successfully"
                );

                ServerMessage::CommandStatus {
                    command_id: cmd.command_id,
                    status: WireCommandStatus::Committed,
                    reason: Some(format!("PURCHASED: Item {} for {} gold", item_id, price)),
                    committed_event_seq: Some(current_tick),
                }
            }
            WireCommandPayload::SetVelocity { .. } => ServerMessage::CommandStatus {
                command_id: cmd.command_id,
                status: WireCommandStatus::Accepted,
                reason: None,
                committed_event_seq: Some(current_tick),
            },
            WireCommandPayload::Custom { .. } | WireCommandPayload::Noop => {
                ServerMessage::CommandStatus {
                    command_id: cmd.command_id,
                    status: WireCommandStatus::Committed,
                    reason: None,
                    committed_event_seq: Some(current_tick),
                }
            }
        }
    }

    /// Advance settlement simulation by one tick (10Hz).
    pub fn step_tick(&self) -> Vec<Uuid> {
        let current_tick = self.tick.fetch_add(1, Ordering::Relaxed) + 1;

        let entities_snapshot = {
            let mut map = self
                .entities
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            // 1. Respawns for fallen entities
            for entity in map.values_mut() {
                if let Some(respawn_tick) = entity.respawn_at_tick {
                    if current_tick >= respawn_tick {
                        let max_hp = entity.state.max_hp.unwrap_or(100);
                        entity.state.hp = Some(max_hp);
                        entity.respawn_at_tick = None;
                        info!(entity_id = %entity.state.id, "Entity respawned");
                    }
                }
            }

            // 2. NPC Autonomous Routines
            // Farmers produce food periodically
            if current_tick % 20 == 0 {
                if let Some(farmer) = map.get_mut(&FARMER_CORIN_ID) {
                    if farmer.state.hp.unwrap_or(0) > 0 {
                        farmer.state.x += farmer.wander_dir;
                        if farmer.state.x > farmer.base_x + 2 || farmer.state.x < farmer.base_x - 2
                        {
                            farmer.wander_dir = -farmer.wander_dir;
                        }
                    }
                }
                if let Some(farmer) = map.get_mut(&FARMER_DELIA_ID) {
                    if farmer.state.hp.unwrap_or(0) > 0 {
                        farmer.state.y += farmer.wander_dir;
                        if farmer.state.y > farmer.base_y + 2 || farmer.state.y < farmer.base_y - 2
                        {
                            farmer.wander_dir = -farmer.wander_dir;
                        }
                    }
                }
            }

            // Guards patrol boundaries
            if current_tick % 10 == 0 {
                if let Some(guard) = map.get_mut(&GUARD_VARICK_ID) {
                    if guard.state.hp.unwrap_or(0) > 0 {
                        guard.state.x += guard.wander_dir;
                        if guard.state.x > guard.base_x + 5 || guard.state.x < guard.base_x - 5 {
                            guard.wander_dir = -guard.wander_dir;
                        }
                    }
                }
                if let Some(guard) = map.get_mut(&GUARD_ROLAND_ID) {
                    if guard.state.hp.unwrap_or(0) > 0 {
                        guard.state.y += guard.wander_dir;
                        if guard.state.y > guard.base_y + 4 || guard.state.y < guard.base_y - 4 {
                            guard.wander_dir = -guard.wander_dir;
                        }
                    }
                }
                if let Some(guard) = map.get_mut(&GUARD_AELYN_ID) {
                    if guard.state.hp.unwrap_or(0) > 0 {
                        guard.state.y += guard.wander_dir;
                        if guard.state.y > guard.base_y + 4 || guard.state.y < guard.base_y - 4 {
                            guard.wander_dir = -guard.wander_dir;
                        }
                    }
                }
            }

            // Wild Boar wandering
            if let Some(boar) = map.get_mut(&WILD_BOAR_ID) {
                if boar.state.hp.unwrap_or(0) > 0 {
                    boar.state.x += boar.wander_dir;
                    if boar.state.x > -6 {
                        boar.wander_dir = -1;
                    } else if boar.state.x < -14 {
                        boar.wander_dir = 1;
                    }
                }
            }

            // 3. Economy: Food consumption & Shortage Trigger
            // Periodically consume food based on population
            if current_tick % 50 == 0 {
                let mut eco = self.economy.write().unwrap_or_else(|p| p.into_inner());
                let mut ledger = self.food_ledger.write().unwrap_or_else(|p| p.into_inner());

                let consumption = (eco.population as i64) * eco.food_consumption_per_person;
                let prev = eco.food_stock;
                let actual = consumption.min(prev);
                if actual > 0 {
                    let _ = ledger.record(
                        current_tick,
                        eco.settlement_id,
                        FoodLedgerOp::Consumption { amount: actual },
                        prev,
                        -actual,
                    );
                    eco.food_stock -= actual;
                }
            }

            // Check food shortage threshold transition:
            // TEST-E2E-001 & TEST-E2E-002: Only false -> true triggers event and announcement
            let mut shortage_triggered = false;
            {
                let mut eco = self.economy.write().unwrap_or_else(|p| p.into_inner());
                if eco.food_stock < eco.shortage_threshold && !eco.shortage_active {
                    eco.shortage_active = true;
                    shortage_triggered = true;
                    info!(
                        current_stock = eco.food_stock,
                        threshold = eco.shortage_threshold,
                        "Edge-triggered food shortage! Dispatching supply caravan request."
                    );
                }
            }

            if shortage_triggered {
                let mut caravan_guard = self
                    .active_caravan
                    .write()
                    .unwrap_or_else(|p| p.into_inner());
                if caravan_guard.is_none() {
                    let caravan = CaravanEventChain::new_shortage_request(
                        format!("caravan-tick-{}", current_tick),
                        1,
                        current_tick,
                        50,
                    );
                    *caravan_guard = Some(caravan);
                    self.replicator.broadcast_message(&ServerMessage::Announcement {
                        title: "Food Shortage Warning".into(),
                        message: "Settlement food stock dropped below threshold! Requesting supply caravan.".into(),
                    });
                }
            }

            // 4. Autonomous Caravan Event Progression
            let mut caravan_resolution_action = None;
            {
                let mut caravan_guard = self
                    .active_caravan
                    .write()
                    .unwrap_or_else(|p| p.into_inner());
                if let Some(ref mut caravan) = *caravan_guard {
                    match &mut caravan.phase {
                        CaravanPhase::Requested {
                            requested_tick,
                            food_amount,
                        } => {
                            if current_tick >= *requested_tick + 5 {
                                let food = *food_amount;
                                caravan.phase = CaravanPhase::Traveling {
                                    depart_tick: current_tick,
                                    eta_tick: current_tick + 20,
                                    food_amount: food,
                                };
                                // Spawn caravan cart on perimeter
                                map.insert(
                                    CARAVAN_CART_ID,
                                    SimulationEntity {
                                        state: EntityStateWire {
                                            id: CARAVAN_CART_ID,
                                            x: -25,
                                            y: -25,
                                            entity_type: "caravan".into(),
                                            hp: Some(100),
                                            max_hp: Some(100),
                                            name: Some("Supply Caravan".into()),
                                        },
                                        respawn_at_tick: None,
                                        wander_dir: 1,
                                        base_x: -25,
                                        base_y: -25,
                                    },
                                );
                                info!("Supply caravan departed and is traveling to settlement");
                            }
                        }
                        CaravanPhase::Traveling {
                            eta_tick,
                            food_amount,
                            ..
                        } => {
                            if let Some(cart) = map.get_mut(&CARAVAN_CART_ID) {
                                // Move cart toward village center (0, 0)
                                if cart.state.x < -5 {
                                    cart.state.x += 1;
                                }
                                if cart.state.y < -5 {
                                    cart.state.y += 1;
                                }
                            }
                            if current_tick >= *eta_tick - 10 {
                                let food = *food_amount;
                                caravan.phase = CaravanPhase::Ambushed {
                                    ambush_tick: current_tick,
                                    timeout_tick: current_tick + 30, // 30 ticks timeout
                                    food_amount: food,
                                    escort_power: 100,
                                    bandit_power: 80,
                                };
                                info!("Supply caravan ambushed by bandits outside settlement!");
                                self.replicator
                                    .broadcast_message(&ServerMessage::Announcement {
                                        title: "Caravan Ambushed!".into(),
                                        message:
                                            "Bandits have ambushed the food caravan on the road!"
                                                .into(),
                                    });
                            }
                        }
                        CaravanPhase::Ambushed {
                            timeout_tick,
                            food_amount,
                            escort_power,
                            bandit_power,
                            ..
                        } => {
                            // Autonomous combat progression between guards and bandits
                            if let Some(guard) = map.get_mut(&GUARD_VARICK_ID) {
                                if guard.state.hp.unwrap_or(0) > 0 {
                                    // Guard attacks bandit
                                    if let Some(bandit) = map.get_mut(&BANDIT_KAGE_ID) {
                                        let hp = bandit.state.hp.unwrap_or(90);
                                        bandit.state.hp = Some((hp - 15).max(0));
                                    }
                                }
                            }

                            // Bandits attack caravan or guards
                            if let Some(bandit) = map.get_mut(&BANDIT_KAGE_ID) {
                                if bandit.state.hp.unwrap_or(0) > 0 {
                                    if let Some(cart) = map.get_mut(&CARAVAN_CART_ID) {
                                        let hp = cart.state.hp.unwrap_or(100);
                                        cart.state.hp = Some((hp - 5).max(0));
                                    }
                                }
                            }

                            // Check resolution conditions (or timeout fallback)
                            let bandit_hp = map
                                .get(&BANDIT_KAGE_ID)
                                .and_then(|b| b.state.hp)
                                .unwrap_or(0);
                            let cart_hp = map
                                .get(&CARAVAN_CART_ID)
                                .and_then(|c| c.state.hp)
                                .unwrap_or(0);

                            if bandit_hp == 0
                                || *escort_power > *bandit_power
                                || current_tick >= *timeout_tick
                            {
                                // Escort success!
                                caravan.phase = CaravanPhase::Resolved {
                                    resolution: CaravanResolution::EscortSucceeded,
                                    resolved_tick: current_tick,
                                    food_delivered: *food_amount,
                                };
                                caravan_resolution_action =
                                    Some(CaravanResolution::EscortSucceeded);
                            } else if cart_hp == 0 {
                                // Raid success!
                                caravan.phase = CaravanPhase::Resolved {
                                    resolution: CaravanResolution::RaidSucceeded,
                                    resolved_tick: current_tick,
                                    food_delivered: 0,
                                };
                                caravan_resolution_action = Some(CaravanResolution::RaidSucceeded);
                            }
                        }
                        CaravanPhase::Resolved { .. } => {
                            // Already resolved, clean up cart
                            map.remove(&CARAVAN_CART_ID);
                        }
                    }
                }
            }

            // Apply resolution side effects to economy and news
            if let Some(resolution) = caravan_resolution_action {
                map.remove(&CARAVAN_CART_ID);
                match resolution {
                    CaravanResolution::EscortSucceeded => {
                        let mut eco = self.economy.write().unwrap_or_else(|p| p.into_inner());
                        let mut ledger =
                            self.food_ledger.write().unwrap_or_else(|p| p.into_inner());
                        let mut stock = self
                            .market_food_stock
                            .write()
                            .unwrap_or_else(|p| p.into_inner());

                        let prev = eco.food_stock;
                        let delivered = 50;
                        let _ = ledger.record(
                            current_tick,
                            eco.settlement_id,
                            FoodLedgerOp::CaravanSupply {
                                amount: delivered,
                                caravan_id: "caravan-res".into(),
                            },
                            prev,
                            delivered,
                        );
                        eco.food_stock += delivered;
                        eco.shortage_active = false;
                        eco.security_index = (eco.security_index + 20).min(500);
                        *stock += 5; // Replenish market stock

                        info!(
                            "Caravan safely escorted into settlement! Stock and security restored."
                        );
                        self.replicator.broadcast_message(&ServerMessage::Announcement {
                            title: "Caravan Delivered".into(),
                            message: "The supply caravan arrived safely! Food stock replenished and market restocked.".into(),
                        });
                    }
                    CaravanResolution::RaidSucceeded => {
                        let mut eco = self.economy.write().unwrap_or_else(|p| p.into_inner());
                        eco.security_index = (eco.security_index - 30).max(0);
                        info!("Caravan raided! Food lost and security diminished.");
                        self.replicator.broadcast_message(&ServerMessage::Announcement {
                            title: "Caravan Lost".into(),
                            message: "Bandits plundered the food caravan! Security in the region has fallen.".into(),
                        });
                    }
                }
            }

            map.values().map(|e| e.state.clone()).collect::<Vec<_>>()
        };

        // Broadcast AOI delta tick to all connected sessions
        self.replicator
            .broadcast_tick(current_tick, &entities_snapshot)
    }
}
