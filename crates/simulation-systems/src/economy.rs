//! Settlement food ledger, market supply/demand, and price discovery.
//!
//! Enforces:
//! - Food stock non-negative invariant (`food_stock >= 0`)
//! - Strict conservation audit via double-entry food ledger
//! - Edge-triggered food shortage transitions with deduplication
//! - Bounded and dampened price oscillations (EMA smoothing)

use game_domain::{DomainError, EntityId};
use serde::{Deserialize, Serialize};
use simulation_core::SimulationEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoodLedgerOp {
    Production {
        amount: i64,
    },
    Consumption {
        amount: i64,
    },
    Purchase {
        amount: i64,
        buyer_id: EntityId,
        unit_price: i64,
    },
    CaravanSupply {
        amount: i64,
        caravan_id: String,
    },
    Spoilage {
        amount: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoodLedgerEntry {
    pub entry_id: u64,
    pub tick: u64,
    pub settlement_id: u64,
    pub op: FoodLedgerOp,
    pub delta: i64,
    pub balance_before: i64,
    pub balance_after: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoodLedger {
    entries: Vec<FoodLedgerEntry>,
    next_entry_id: u64,
}

impl FoodLedger {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_entry_id: 1,
        }
    }

    pub fn entries(&self) -> &[FoodLedgerEntry] {
        &self.entries
    }

    /// Record a delta in the ledger, enforcing `balance_after >= 0` invariant.
    pub fn record(
        &mut self,
        tick: u64,
        settlement_id: u64,
        op: FoodLedgerOp,
        current_balance: i64,
        delta: i64,
    ) -> Result<i64, DomainError> {
        let new_balance = current_balance + delta;
        if new_balance < 0 {
            return Err(DomainError::NegativeFoodStock {
                settlement_id,
                stock: new_balance,
            });
        }

        let entry = FoodLedgerEntry {
            entry_id: self.next_entry_id,
            tick,
            settlement_id,
            op,
            delta,
            balance_before: current_balance,
            balance_after: new_balance,
        };
        self.next_entry_id += 1;
        self.entries.push(entry);

        Ok(new_balance)
    }

    /// Audit conservation: checks that all entries form a contiguous, non-negative balance chain.
    pub fn audit_conservation(&self) -> bool {
        let mut prev_balance: Option<i64> = None;
        for entry in &self.entries {
            if entry.balance_after < 0 {
                return false;
            }
            if prev_balance.is_some_and(|prev| entry.balance_before != prev) {
                return false;
            }
            if entry.balance_after != entry.balance_before + entry.delta {
                return false;
            }
            prev_balance = Some(entry.balance_after);
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementEconomy {
    pub settlement_id: u64,
    pub population: u32,
    pub food_stock: i64,
    pub food_capacity: i64,
    pub security_index: i32,
    pub treasury: i64,
    pub active_farms: u32,
    pub base_yield_per_farm: i64,
    pub food_consumption_per_person: i64,
    pub target_stock: i64,
    pub base_price: i64,
    pub min_price: i64,
    pub max_price: i64,
    pub current_price: i64,
    pub price_ema_factor: i64,
    pub shortage_active: bool,
    pub shortage_threshold: i64,
}

impl SettlementEconomy {
    pub fn new_default(settlement_id: u64) -> Self {
        Self::new(settlement_id, 20, 100, 500, 100, 10, 30)
    }

    pub fn new(
        settlement_id: u64,
        population: u32,
        initial_stock: i64,
        food_capacity: i64,
        target_stock: i64,
        base_price: i64,
        shortage_threshold: i64,
    ) -> Self {
        Self {
            settlement_id,
            population,
            food_stock: initial_stock.max(0),
            food_capacity: food_capacity.max(1),
            security_index: 70,
            treasury: 1000,
            active_farms: 5,
            base_yield_per_farm: 10,
            food_consumption_per_person: 1,
            target_stock: target_stock.max(1),
            base_price: base_price.max(1),
            min_price: (base_price / 2).max(1),
            max_price: base_price * 4,
            current_price: base_price.max(1),
            price_ema_factor: 25, // 25% EMA adjustment per step
            shortage_active: initial_stock < shortage_threshold,
            shortage_threshold,
        }
    }

    pub fn step_settlement_economy(
        &mut self,
        tick: u64,
        weather_scale: i64,
        ledger: &mut FoodLedger,
    ) -> Vec<SimulationEvent> {
        self.step_economy(tick, weather_scale, ledger)
    }

    /// Progress one economy cycle (e.g. daily/hourly or 1-minute aggregation window).
    ///
    /// Weather scale is 100 for normal (1.0x), 50 for drought (0.5x), etc.
    pub fn step_economy(
        &mut self,
        tick: u64,
        weather_scale: i64,
        ledger: &mut FoodLedger,
    ) -> Vec<SimulationEvent> {
        let mut events = Vec::new();

        // 1. Calculate production & consumption
        let raw_production =
            (self.active_farms as i64 * self.base_yield_per_farm * weather_scale) / 100;
        let production = raw_production.max(0);
        let consumption = (self.population as i64 * self.food_consumption_per_person).max(0);

        // 2. Record production into ledger (bounded by capacity)
        let room = (self.food_capacity - self.food_stock).max(0);
        let actual_production = production.min(room);
        if actual_production > 0 {
            let res = ledger.record(
                tick,
                self.settlement_id,
                FoodLedgerOp::Production {
                    amount: actual_production,
                },
                self.food_stock,
                actual_production,
            );
            if let Ok(new_bal) = res {
                self.food_stock = new_bal;
            }
        }

        // 3. Record consumption into ledger (bounded by available stock to ensure stock >= 0)
        let actual_consumption = consumption.min(self.food_stock);
        if actual_consumption > 0 {
            let res = ledger.record(
                tick,
                self.settlement_id,
                FoodLedgerOp::Consumption {
                    amount: actual_consumption,
                },
                self.food_stock,
                -actual_consumption,
            );
            if let Ok(new_bal) = res {
                self.food_stock = new_bal;
            }
        }

        // 4. Update dynamic price based on scarcity relative to target_stock
        let target = self.target_stock.max(1);
        let scarcity_pct = ((target - self.food_stock) * 100) / target;
        let clamped_scarcity = scarcity_pct.clamp(-100, 100);

        let raw_price = self.base_price + (self.base_price * clamped_scarcity) / 100;
        let bounded_target_price = raw_price.clamp(self.min_price, self.max_price);

        // EMA smoothing: current = (current * (100 - ema) + target * ema) / 100
        let ema = self.price_ema_factor.clamp(1, 100);
        self.current_price =
            ((self.current_price * (100 - ema)) + (bounded_target_price * ema)) / 100;
        self.current_price = self.current_price.clamp(self.min_price, self.max_price);

        // 5. Edge-triggered shortage check:
        // Transition false -> true emits FoodShortage event.
        // If already in shortage, duplicate emission is suppressed.
        let is_now_shortage = self.food_stock < self.shortage_threshold;
        if !self.shortage_active && is_now_shortage {
            self.shortage_active = true;
            let deficit = self.shortage_threshold - self.food_stock;
            events.push(SimulationEvent::FoodShortage {
                settlement_id: Some(EntityId::from_u128(self.settlement_id as u128)),
                deficit,
            });
        } else if self.shortage_active && !is_now_shortage {
            self.shortage_active = false;
        }

        events
    }

    /// Atomically execute a food purchase by an entity/player.
    ///
    /// Fails with `OutOfStock` if available stock is less than requested quantity.
    pub fn execute_purchase(
        &mut self,
        tick: u64,
        buyer_id: EntityId,
        quantity: i64,
        ledger: &mut FoodLedger,
    ) -> Result<SimulationEvent, DomainError> {
        if quantity <= 0 {
            return Err(DomainError::InvalidCommand {
                reason: "purchase quantity must be greater than zero".to_string(),
            });
        }

        if self.food_stock < quantity {
            return Err(DomainError::OutOfStock {
                sku: "FOOD_RATION".to_string(),
            });
        }

        let total_price = self.current_price * quantity;

        let new_stock = ledger.record(
            tick,
            self.settlement_id,
            FoodLedgerOp::Purchase {
                amount: quantity,
                buyer_id,
                unit_price: self.current_price,
            },
            self.food_stock,
            -quantity,
        )?;

        self.food_stock = new_stock;
        self.treasury += total_price;

        Ok(SimulationEvent::PurchaseExecuted {
            buyer_id,
            item_id: 1, // Food ration item id
            price: total_price,
        })
    }

    /// Restock food via caravan delivery.
    pub fn apply_caravan_resupply(
        &mut self,
        tick: u64,
        caravan_id: &str,
        amount: i64,
        ledger: &mut FoodLedger,
    ) -> Result<(), DomainError> {
        if amount <= 0 {
            return Ok(());
        }

        let room = (self.food_capacity - self.food_stock).max(0);
        let actual_supplied = amount.min(room);

        if actual_supplied > 0 {
            let new_stock = ledger.record(
                tick,
                self.settlement_id,
                FoodLedgerOp::CaravanSupply {
                    amount: actual_supplied,
                    caravan_id: caravan_id.to_string(),
                },
                self.food_stock,
                actual_supplied,
            )?;
            self.food_stock = new_stock;
        }

        if self.food_stock >= self.shortage_threshold {
            self.shortage_active = false;
        }

        Ok(())
    }
}
