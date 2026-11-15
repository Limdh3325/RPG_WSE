//! Dynamic rule AST, safe evaluation engine, and validation rules.
//!
//! Enforces:
//! - Strict domain rule validator with no arbitrary execution
//! - Rejection of duplicate rule IDs (`DomainError::DuplicateRule`)
//! - Bounded AST recursion depth (max depth 10)
//! - Edge transition evaluation (`false -> true`) preventing event spam

use game_domain::DomainError;
use serde::{Deserialize, Serialize};
use simulation_core::SimulationEvent;
use std::collections::HashMap;

pub const MAX_RULE_AST_DEPTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricKey {
    FoodStock(u64),      // settlement_id
    SecurityIndex(u64),  // settlement_id
    MarketPrice(u64),    // settlement_id
    ShortageActive(u64), // settlement_id (1 if active, 0 if false)
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl ComparisonOp {
    pub fn evaluate(&self, lhs: i64, rhs: i64) -> bool {
        match self {
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Gt => lhs > rhs,
            Self::Ge => lhs >= rhs,
            Self::Eq => lhs == rhs,
            Self::Ne => lhs != rhs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionAST {
    MetricThreshold {
        metric: MetricKey,
        op: ComparisonOp,
        value: i64,
    },
    EdgeTransition {
        metric: MetricKey,
        op: ComparisonOp,
        value: i64,
    },
    And(Vec<ConditionAST>),
    Or(Vec<ConditionAST>),
    Not(Box<ConditionAST>),
    CooldownReady {
        cooldown_ticks: u64,
    },
}

impl ConditionAST {
    pub fn validate_depth(&self, current_depth: usize) -> Result<(), DomainError> {
        if current_depth > MAX_RULE_AST_DEPTH {
            return Err(DomainError::InvalidRuleAst {
                reason: format!("AST depth exceeded maximum allowed ({MAX_RULE_AST_DEPTH})"),
            });
        }
        match self {
            Self::And(subs) | Self::Or(subs) => {
                if subs.is_empty() {
                    return Err(DomainError::InvalidRuleAst {
                        reason: "And/Or condition cannot be empty".to_string(),
                    });
                }
                for sub in subs {
                    sub.validate_depth(current_depth + 1)?;
                }
            }
            Self::Not(sub) => {
                sub.validate_depth(current_depth + 1)?;
            }
            Self::MetricThreshold { .. }
            | Self::EdgeTransition { .. }
            | Self::CooldownReady { .. } => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionAST {
    EmitEvent(SimulationEvent),
    EmitNews {
        headline: String,
    },
    DispatchCaravan {
        caravan_id: String,
        settlement_id: u64,
        food_amount: i64,
    },
    AdjustSecurity {
        settlement_id: u64,
        delta: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicRule {
    pub id: String,
    pub name: String,
    pub condition: ConditionAST,
    pub actions: Vec<ActionAST>,
    pub cooldown_ticks: u64,
    pub last_triggered_tick: Option<u64>,
}

impl DynamicRule {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.trim().is_empty() {
            return Err(DomainError::InvalidRuleAst {
                reason: "rule id cannot be empty".to_string(),
            });
        }
        if self.actions.is_empty() {
            return Err(DomainError::InvalidRuleAst {
                reason: "rule must contain at least one action".to_string(),
            });
        }
        self.condition.validate_depth(1)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetricContext {
    values: HashMap<MetricKey, i64>,
}

impl MetricContext {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: MetricKey, value: i64) {
        self.values.insert(key, value);
    }

    pub fn get(&self, key: &MetricKey) -> i64 {
        self.values.get(key).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DynamicRuleRegistry {
    rules: HashMap<String, DynamicRule>,
    rule_order: Vec<String>,
}

impl DynamicRuleRegistry {
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
            rule_order: Vec::new(),
        }
    }

    pub fn register_rule(&mut self, rule: DynamicRule) -> Result<(), DomainError> {
        rule.validate()?;

        if self.rules.contains_key(&rule.id) {
            return Err(DomainError::DuplicateRule { rule_id: rule.id });
        }

        let id = rule.id.clone();
        self.rules.insert(id.clone(), rule);
        self.rule_order.push(id);
        Ok(())
    }

    pub fn get_rule(&self, id: &str) -> Option<&DynamicRule> {
        self.rules.get(id)
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    fn evaluate_condition(
        condition: &ConditionAST,
        current_tick: u64,
        last_triggered: Option<u64>,
        default_cooldown: u64,
        prev_ctx: &MetricContext,
        curr_ctx: &MetricContext,
    ) -> bool {
        match condition {
            ConditionAST::MetricThreshold { metric, op, value } => {
                let curr_val = curr_ctx.get(metric);
                op.evaluate(curr_val, *value)
            }
            ConditionAST::EdgeTransition { metric, op, value } => {
                let prev_val = prev_ctx.get(metric);
                let curr_val = curr_ctx.get(metric);
                let prev_sat = op.evaluate(prev_val, *value);
                let curr_sat = op.evaluate(curr_val, *value);
                // Edge transition: must be false before, and true now!
                !prev_sat && curr_sat
            }
            ConditionAST::And(subs) => subs.iter().all(|sub| {
                Self::evaluate_condition(
                    sub,
                    current_tick,
                    last_triggered,
                    default_cooldown,
                    prev_ctx,
                    curr_ctx,
                )
            }),
            ConditionAST::Or(subs) => subs.iter().any(|sub| {
                Self::evaluate_condition(
                    sub,
                    current_tick,
                    last_triggered,
                    default_cooldown,
                    prev_ctx,
                    curr_ctx,
                )
            }),
            ConditionAST::Not(sub) => !Self::evaluate_condition(
                sub,
                current_tick,
                last_triggered,
                default_cooldown,
                prev_ctx,
                curr_ctx,
            ),
            ConditionAST::CooldownReady { cooldown_ticks } => {
                let cd = if *cooldown_ticks > 0 {
                    *cooldown_ticks
                } else {
                    default_cooldown
                };
                last_triggered.is_none_or(|last| current_tick >= last + cd)
            }
        }
    }

    /// Evaluates registered rules in deterministic order and triggers satisfied actions.
    pub fn evaluate(
        &mut self,
        current_tick: u64,
        prev_ctx: &MetricContext,
        curr_ctx: &MetricContext,
    ) -> Vec<ActionAST> {
        let mut triggered_actions = Vec::new();

        for rule_id in &self.rule_order {
            if let Some(rule) = self.rules.get_mut(rule_id) {
                // Check cooldown
                if rule
                    .last_triggered_tick
                    .is_some_and(|last| current_tick < last + rule.cooldown_ticks)
                {
                    continue;
                }

                // Evaluate condition
                let condition_met = Self::evaluate_condition(
                    &rule.condition,
                    current_tick,
                    rule.last_triggered_tick,
                    rule.cooldown_ticks,
                    prev_ctx,
                    curr_ctx,
                );

                if condition_met {
                    rule.last_triggered_tick = Some(current_tick);
                    triggered_actions.extend(rule.actions.clone());
                }
            }
        }

        triggered_actions
    }
}
