//! Deterministic tick runtime; TASK-003.
//!
//! Provides the core simulation loop, clock, deterministic PRNG stream,
//! phase pipeline, command admission, and reproducible state hashing.

pub mod clock;
pub mod command;
pub mod coordinator;
pub mod engine;
pub mod event;
pub mod hook;
pub mod pathfinding;
pub mod phase;
pub mod rng;
pub mod worker;

pub use clock::{ClockStatus, SimulationClock};
pub use command::{
    CommandEnvelope, CommandPayload, CommandQueue, CommandStatus, DeduplicationOutcome,
    IngressCommand, SUPPORTED_COMMAND_SCHEMA_VERSION, TransientCommandDeduplicator,
};
pub use coordinator::{CoordinatorError, EpochValidator, RegionCoordinator, RouteError};
pub use engine::{SimulationEngine, SimulationSummary, TickOutput};
pub use event::{
    EventBudget, EventEnvelope, EventRule, RuleAction, RuleEngine, SUPPORTED_EVENT_SCHEMA_VERSION,
    SimulationEvent,
};
pub use hook::{NoopSimulationHooks, SimulationPhaseHooks};
pub use phase::TickPhase;
pub use rng::DeterministicRng;
pub use worker::{
    COMMAND_CHANNEL_CAPACITY, CONTROL_CHANNEL_CAPACITY, ControlMessage, DEFAULT_TICK_INTERVAL_MS,
    EVENT_CHANNEL_CAPACITY, LAG_THRESHOLD_MS, MAX_ACCUMULATED_DRIFT_MS, PERSISTENCE_ACK_CAPACITY,
    PersistenceAck, RegionHandle, RegionHealth, RegionStatus, RegionWorker, RegionWorkerStatus,
    SimulationMode,
};
