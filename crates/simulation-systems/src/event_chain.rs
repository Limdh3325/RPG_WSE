//! Dynamic world event chains: shortage -> caravan dispatch -> ambush -> resolution.
//!
//! Enforces:
//! - Autonomous progression without requiring player input (DoD)
//! - Clean termination via NPC actions or timeout fallback (no stuck events)
//! - Integration with food ledger, security index, faction standings, and news announcements

use crate::economy::{FoodLedger, SettlementEconomy};
use crate::faction::{FactionId, FactionManager};
use game_domain::EntityId;
use serde::{Deserialize, Serialize};
use simulation_core::SimulationEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerCaravanIntervention {
    EscortSupport { combat_contribution: i32 },
    JoinRaid { raid_contribution: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaravanResolution {
    EscortSucceeded,
    RaidSucceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaravanPhase {
    Requested {
        requested_tick: u64,
        food_amount: i64,
    },
    Traveling {
        depart_tick: u64,
        eta_tick: u64,
        food_amount: i64,
    },
    Ambushed {
        ambush_tick: u64,
        timeout_tick: u64,
        food_amount: i64,
        escort_power: i32,
        bandit_power: i32,
    },
    Resolved {
        resolution: CaravanResolution,
        resolved_tick: u64,
        food_delivered: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaravanEventChain {
    pub caravan_id: String,
    pub settlement_id: u64,
    pub phase: CaravanPhase,
    pub news_log: Vec<String>,
}

impl CaravanEventChain {
    pub fn new_shortage_request(
        caravan_id: String,
        settlement_id: u64,
        requested_tick: u64,
        food_amount: i64,
    ) -> Self {
        Self {
            caravan_id,
            settlement_id,
            phase: CaravanPhase::Requested {
                requested_tick,
                food_amount,
            },
            news_log: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.phase, CaravanPhase::Resolved { .. })
    }

    /// Advances the event chain by one simulation tick.
    ///
    /// Even if `player_intervention` is None (player ignores the event),
    /// the caravan will autonomously travel, trigger ambush based on security,
    /// and resolve cleanly via timeout/NPC combat resolution.
    pub fn step(
        &mut self,
        current_tick: u64,
        settlement: &mut SettlementEconomy,
        faction_mgr: &mut FactionManager,
        ledger: &mut FoodLedger,
        player_actor: Option<EntityId>,
        player_intervention: Option<PlayerCaravanIntervention>,
    ) -> Vec<SimulationEvent> {
        let mut events = Vec::new();

        match &mut self.phase {
            CaravanPhase::Requested {
                requested_tick: _,
                food_amount,
            } => {
                // Dispatch caravan after request
                let amt = *food_amount;
                let eta = current_tick + 5; // 5 ticks travel time
                self.phase = CaravanPhase::Traveling {
                    depart_tick: current_tick,
                    eta_tick: eta,
                    food_amount: amt,
                };
                self.news_log.push(format!(
                    "상단 보급 수송대({caravan})가 정착지 #{settlement}로 출발했습니다. (식량 {amt} 수송)",
                    caravan = self.caravan_id,
                    settlement = self.settlement_id
                ));
                events.push(SimulationEvent::CaravanDispatched {
                    caravan_id: self.caravan_id.clone(),
                    settlement_id: self.settlement_id,
                    food_amount: amt,
                });
            }

            CaravanPhase::Traveling {
                depart_tick: _,
                eta_tick,
                food_amount,
            } => {
                let amt = *food_amount;
                // Check if ambush is triggered mid-way (tick >= eta - 2) or if security is low
                let mid_journey = current_tick >= *eta_tick - 2;
                let vulnerable = settlement.security_index < 80;

                if mid_journey && vulnerable {
                    // Trigger Bandit Ambush!
                    let timeout = current_tick + 6; // 6 ticks to resolve or timeout fallback
                    let escort_pwr = 40 + (settlement.security_index / 2);
                    let bandit_pwr = 60;

                    self.phase = CaravanPhase::Ambushed {
                        ambush_tick: current_tick,
                        timeout_tick: timeout,
                        food_amount: amt,
                        escort_power: escort_pwr,
                        bandit_power: bandit_pwr,
                    };

                    self.news_log.push(format!(
                        "[긴급] 수송대({caravan})가 협곡 도로에서 도적단의 기습을 받았습니다!",
                        caravan = self.caravan_id
                    ));

                    events.push(SimulationEvent::CaravanAmbushed {
                        caravan_id: self.caravan_id.clone(),
                        road_id: "GorgePass".to_string(),
                        threat_level: 3,
                    });
                } else if current_tick >= *eta_tick {
                    // Safe arrival without ambush
                    let _ = settlement.apply_caravan_resupply(
                        current_tick,
                        &self.caravan_id,
                        amt,
                        ledger,
                    );
                    settlement.security_index = (settlement.security_index + 2).min(100);

                    self.phase = CaravanPhase::Resolved {
                        resolution: CaravanResolution::EscortSucceeded,
                        resolved_tick: current_tick,
                        food_delivered: amt,
                    };

                    self.news_log.push(format!(
                        "수송대({caravan})가 안전하게 정착지에 도착하여 식량을 보급했습니다.",
                        caravan = self.caravan_id
                    ));

                    events.push(SimulationEvent::CaravanResolved {
                        caravan_id: self.caravan_id.clone(),
                        outcome: "escort_succeeded".to_string(),
                        food_delivered: amt,
                    });
                }
            }

            CaravanPhase::Ambushed {
                ambush_tick: _,
                timeout_tick,
                food_amount,
                escort_power,
                bandit_power,
            } => {
                let amt = *food_amount;
                let mut resolution: Option<CaravanResolution> = None;

                // 1. Process player intervention if present
                if let Some(intervention) = player_intervention {
                    match intervention {
                        PlayerCaravanIntervention::EscortSupport {
                            combat_contribution,
                        } => {
                            *escort_power += combat_contribution;
                            if let Some(actor) = player_actor {
                                faction_mgr.adjust_reputation(actor, FactionId::Settlers, 15);
                                faction_mgr.adjust_reputation(actor, FactionId::Bandits, -20);
                            }
                            if *escort_power >= *bandit_power {
                                resolution = Some(CaravanResolution::EscortSucceeded);
                            }
                        }
                        PlayerCaravanIntervention::JoinRaid { raid_contribution } => {
                            *bandit_power += raid_contribution;
                            if let Some(actor) = player_actor {
                                faction_mgr.adjust_reputation(actor, FactionId::Settlers, -25);
                                faction_mgr.adjust_reputation(actor, FactionId::Bandits, 20);
                            }
                            if *bandit_power > *escort_power {
                                resolution = Some(CaravanResolution::RaidSucceeded);
                            }
                        }
                    }
                }

                // 2. Autonomous resolution on timeout (DoD: player ignores/spectates)
                if resolution.is_none() && current_tick >= *timeout_tick {
                    // Guard vs Bandit autonomous resolution based on powers and security
                    if *escort_power >= *bandit_power || settlement.security_index >= 60 {
                        resolution = Some(CaravanResolution::EscortSucceeded);
                    } else {
                        resolution = Some(CaravanResolution::RaidSucceeded);
                    }
                }

                // 3. Apply outcome if resolved
                if let Some(res) = resolution {
                    match res {
                        CaravanResolution::EscortSucceeded => {
                            let _ = settlement.apply_caravan_resupply(
                                current_tick,
                                &self.caravan_id,
                                amt,
                                ledger,
                            );
                            settlement.security_index = (settlement.security_index + 5).min(100);

                            self.news_log.push(format!(
                                "수송대({caravan})가 도적단을 격퇴하고 정착지 #{settlement}에 무사히 식량을 보급했습니다.",
                                caravan = self.caravan_id,
                                settlement = self.settlement_id
                            ));

                            self.phase = CaravanPhase::Resolved {
                                resolution: CaravanResolution::EscortSucceeded,
                                resolved_tick: current_tick,
                                food_delivered: amt,
                            };

                            events.push(SimulationEvent::CaravanResolved {
                                caravan_id: self.caravan_id.clone(),
                                outcome: "escort_succeeded".to_string(),
                                food_delivered: amt,
                            });
                            events.push(SimulationEvent::SettlementSecurityChanged {
                                settlement_id: self.settlement_id,
                                new_security_index: settlement.security_index,
                            });
                        }
                        CaravanResolution::RaidSucceeded => {
                            settlement.security_index = (settlement.security_index - 10).max(0);

                            self.news_log.push(format!(
                                "[약탈 피해] 수송대({caravan})가 도적단에 의해 약탈당했습니다! 식량 보급 실패.",
                                caravan = self.caravan_id
                            ));

                            self.phase = CaravanPhase::Resolved {
                                resolution: CaravanResolution::RaidSucceeded,
                                resolved_tick: current_tick,
                                food_delivered: 0,
                            };

                            events.push(SimulationEvent::CaravanResolved {
                                caravan_id: self.caravan_id.clone(),
                                outcome: "raid_succeeded".to_string(),
                                food_delivered: 0,
                            });
                            events.push(SimulationEvent::SettlementSecurityChanged {
                                settlement_id: self.settlement_id,
                                new_security_index: settlement.security_index,
                            });
                        }
                    }
                }
            }

            CaravanPhase::Resolved { .. } => {
                // Event chain has concluded, no-op
            }
        }

        events
    }
}
