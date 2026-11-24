use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("SQLx database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error(
        "Region lease fencing failed: region {region_id}, expected epoch {expected_epoch}, actual epoch {actual_epoch:?}"
    )]
    LeaseFencingFailed {
        region_id: Uuid,
        expected_epoch: i64,
        actual_epoch: Option<i64>,
    },

    #[error("Command payload hash mismatch for command {command_id}")]
    CommandHashMismatch { command_id: Uuid },

    #[error(
        "Insufficient funds in wallet {wallet_id}: current balance {balance}, required {required}"
    )]
    InsufficientFunds {
        wallet_id: Uuid,
        balance: i64,
        required: i64,
    },

    #[error(
        "Insufficient market stock for settlement {settlement_id}, sku {sku}: available {available}, required {required}"
    )]
    InsufficientStock {
        settlement_id: Uuid,
        sku: String,
        available: i64,
        required: i64,
    },

    #[error("Entity or record not found: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Injected failpoint error: {0}")]
    FailpointInjected(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Snapshot corrupted: {snapshot_id}, reason: {reason}")]
    CorruptedSnapshot { snapshot_id: Uuid, reason: String },

    #[error("Incompatible engine/schema/rules version: {reason}")]
    IncompatibleVersion { reason: String },

    #[error("Recovery failed: {0}")]
    RecoveryFailed(String),

    #[error("Admission paused or rejected due to DB health status")]
    AdmissionPaused,
}
