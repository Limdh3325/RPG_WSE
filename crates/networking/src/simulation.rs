use crate::gateway::InboundCommand;
use crate::replicator::RegionReplicator;
use crate::session::SharedSession;
use shared_protocol::{EntityStateWire, ServerMessage, WireCommandPayload, WireCommandStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

pub const DEFAULT_SETTLEMENT_REGION_ID: Uuid = Uuid::from_u128(1);
pub const ELDER_ELDA_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000101);
pub const MERCHANT_BORIN_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000102);
pub const TRAINING_DUMMY_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000103);
pub const WILD_BOAR_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000104);

#[derive(Debug, Clone)]
pub struct SimulationEntity {
    pub state: EntityStateWire,
    pub respawn_at_tick: Option<u64>,
    pub wander_dir: i32,
}

pub struct SettlementSimulation {
    pub region_id: Uuid,
    pub entities: Arc<RwLock<HashMap<Uuid, SimulationEntity>>>,
    /// Persisted player state across disconnects: (x, y, hp, max_hp)
    pub persistent_player_states: Arc<RwLock<HashMap<Uuid, (i32, i32, i32, i32)>>>,
    pub tick: AtomicU64,
    pub replicator: Arc<RegionReplicator>,
}

impl SettlementSimulation {
    pub fn new(region_id: Uuid, replicator: Arc<RegionReplicator>) -> Self {
        let mut map = HashMap::new();

        // 1. Elder Elda (Town elder NPC)
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
            },
        );

        // 2. Merchant Borin (Shopkeeper NPC)
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
            },
        );

        // 3. Training Dummy (Monster for combat validation)
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
            },
        );

        // 4. Wild Boar (Patrolling monster)
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
            },
        );

        Self {
            region_id,
            entities: Arc::new(RwLock::new(map)),
            persistent_player_states: Arc::new(RwLock::new(HashMap::new())),
            tick: AtomicU64::new(1),
            replicator,
        }
    }

    pub fn current_tick(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }

    pub fn register_player(&self, actor_id: Uuid, name: Option<String>) -> EntityStateWire {
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
            WireCommandPayload::SetVelocity { .. } => ServerMessage::CommandStatus {
                command_id: cmd.command_id,
                status: WireCommandStatus::Accepted,
                reason: None,
                committed_event_seq: Some(current_tick),
            },
            WireCommandPayload::Purchase { .. }
            | WireCommandPayload::Custom { .. }
            | WireCommandPayload::Noop => ServerMessage::CommandStatus {
                command_id: cmd.command_id,
                status: WireCommandStatus::Committed,
                reason: None,
                committed_event_seq: Some(current_tick),
            },
        }
    }

    pub fn step_tick(&self) -> Vec<Uuid> {
        let current_tick = self.tick.fetch_add(1, Ordering::Relaxed) + 1;

        let entities_snapshot = {
            let mut map = self
                .entities
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            // 1. Check respawns
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

            // 2. Light autonomous wandering for Wild Boar
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

            map.values().map(|e| e.state.clone()).collect::<Vec<_>>()
        };

        // Broadcast AOI delta tick to all connected sessions
        self.replicator
            .broadcast_tick(current_tick, &entities_snapshot)
    }
}
