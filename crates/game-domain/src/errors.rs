//! Domain errors across regions, entities, and validation rules.

use crate::id::EntityId;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DomainError {
    #[error("duplicate entity id: {0}")]
    DuplicateEntityId(EntityId),

    #[error("duplicate runtime entity handle: {0}")]
    DuplicateEntityHandle(String),

    #[error("entity not found: {0}")]
    EntityNotFound(EntityId),

    #[error("entity component missing: {0}")]
    ComponentMissing(&'static str),

    #[error("invalid position: ({x}, {y}) out of bounds ({min_x}..={max_x}, {min_y}..={max_y})")]
    OutOfBoundsPosition {
        x: i64,
        y: i64,
        min_x: i64,
        max_x: i64,
        min_y: i64,
        max_y: i64,
    },

    #[error("position on unwalkable/blocked tile: ({x}, {y})")]
    BlockedPosition { x: i64, y: i64 },

    #[error("invalid health: current {current} > max {max} or max <= 0")]
    InvalidHealth { current: i32, max: i32 },

    #[error("invalid needs parameter: {0}")]
    InvalidNeeds(&'static str),

    #[error("invalid personality value: {0}")]
    InvalidPersonality(&'static str),

    #[error("internal ecs error: {0}")]
    EcsError(String),

    #[error(
        "idempotency conflict for command {command_id}: existing hash {existing_hash:?}, incoming hash {incoming_hash:?}"
    )]
    IdempotencyConflict {
        command_id: Uuid,
        existing_hash: [u8; 32],
        incoming_hash: [u8; 32],
    },

    #[error("unsupported schema version: supported {supported}, received {received}")]
    InvalidSchemaVersion { supported: u32, received: u32 },

    #[error("event budget exceeded for tick {tick}: max {max_events}, requested {requested}")]
    EventBudgetExceeded {
        tick: u64,
        max_events: usize,
        requested: usize,
    },

    #[error("event rule cycle detected on trigger '{trigger_event}' at depth {cycle_depth}")]
    RuleCycleDetected {
        trigger_event: String,
        cycle_depth: usize,
    },

    #[error("invalid command state transition from {from:?} to {to:?}")]
    InvalidCommandStateTransition { from: String, to: String },

    #[error("invalid command: {reason}")]
    InvalidCommand { reason: String },

    #[error(
        "negative food stock invariant violated: settlement {settlement_id}, resulting stock {stock}"
    )]
    NegativeFoodStock { settlement_id: u64, stock: i64 },

    #[error("duplicate rule registered: {rule_id}")]
    DuplicateRule { rule_id: String },

    #[error("invalid rule ast: {reason}")]
    InvalidRuleAst { reason: String },

    #[error("item out of stock for sku: {sku}")]
    OutOfStock { sku: String },
}
