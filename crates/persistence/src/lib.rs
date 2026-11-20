//! SQLx persistence boundary, durable transactions, snapshots, and recovery for RPG WSE.

pub mod error;
pub mod lease;
pub mod migrator;
pub mod models;
pub mod outbox;
pub mod recovery;
pub mod snapshot;
pub mod writer;

pub use error::PersistenceError;
pub use lease::{acquire_or_renew_lease, verify_lease_fencing_tx};
pub use migrator::{apply_migration, INITIAL_SCHEMA_SQL};
pub use models::{
    CommandReceiptRecord, DurableEvent, DurableIntent, DurableOperation, PersistenceAck,
    ReceiptStatus,
};
pub use outbox::{EventDeduplicator, OutboxPublisher, OutboxRecord, SqlxEventDeduplicator};
pub use recovery::{RecoveredRegionState, RecoveryManager};
pub use snapshot::{
    ScheduledActionDto, SnapshotDto, SnapshotManager, CURRENT_ENGINE_VERSION,
    CURRENT_RULES_VERSION, CURRENT_SCHEMA_VERSION,
};
pub use writer::DurableWriter;
