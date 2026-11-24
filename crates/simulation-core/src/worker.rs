//! Region Worker thread and lifecycle management for authoritative simulation.
//!
//! Each active region runs on a dedicated OS thread isolated from the async Tokio runtime.
//! Input channels are strictly bounded to prevent unbounded memory growth under backpressure:
//! - Player Command channel: capacity 4096 (drops with ServerBusy on saturation)
//! - Persistence Ack channel: capacity 1024 (prioritized settlement path)
//! - Control channel: capacity 64 (lifecycle and management)
//!
//! Ticks are fixed at 10Hz (100ms interval) with drift compensation and lag detection.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ecs_components::grid::MapGrid;
use ecs_components::region_world::RegionWorld;
use game_domain::RegionId;
pub use game_domain::{RegionHealth, RegionStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;

use crate::command::CommandEnvelope;
use crate::engine::{DurableLootIntent, SimulationEngine, TickOutput};

/// Bounded channel capacities matching system specification.
pub const COMMAND_CHANNEL_CAPACITY: usize = 4096;
pub const PERSISTENCE_ACK_CAPACITY: usize = 1024;
pub const CONTROL_CHANNEL_CAPACITY: usize = 64;
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Default tick interval for 10Hz simulation (100ms per tick).
pub const DEFAULT_TICK_INTERVAL_MS: u64 = 100;
/// Threshold exceeding tick duration to trigger lag warning/degradation.
pub const LAG_THRESHOLD_MS: u64 = 100;
/// Maximum accumulated drift (500ms = 5 ticks) before skipping/resetting baseline.
pub const MAX_ACCUMULATED_DRIFT_MS: u64 = 500;

/// Simulation execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationMode {
    Active,
    Background,
    Paused,
}

/// Acknowledgment of a durable transaction or loot intent settlement from PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistenceAck {
    pub intent_id: uuid::Uuid,
    pub success: bool,
    pub reason: Option<String>,
}

/// Control message sent from Coordinator/Router to Region Worker.
#[derive(Debug)]
pub enum ControlMessage {
    Pause,
    Resume,
    SetMode(SimulationMode),
    Shutdown,
    ShutdownDrain(oneshot::Sender<Vec<DurableLootIntent>>),
    DrainIntents(oneshot::Sender<Vec<DurableLootIntent>>),
    SetTestTickDelay(Duration),
    InspectStatus(oneshot::Sender<RegionWorkerStatus>),
}

/// Point-in-time status report of a region worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionWorkerStatus {
    pub region_id: RegionId,
    pub epoch: u64,
    pub current_tick: u64,
    pub status: RegionStatus,
    pub health: RegionHealth,
    pub last_tick_duration_ms: u64,
    pub consecutive_lagged_ticks: u32,
}

/// A handle to an active Region Worker used by Coordinator and Router.
#[derive(Clone)]
pub struct RegionHandle {
    pub region_id: RegionId,
    pub epoch: u64,
    pub cmd_tx: Sender<CommandEnvelope>,
    pub ack_tx: Sender<PersistenceAck>,
    pub control_tx: Sender<ControlMessage>,
    pub status: Arc<RwLock<RegionStatus>>,
    pub health: Arc<RwLock<RegionHealth>>,
    pub is_shutdown: Arc<AtomicBool>,
    drained_intents: Arc<RwLock<Vec<DurableLootIntent>>>,
}

impl RegionHandle {
    /// Inspect current lifecycle state.
    pub fn status(&self) -> RegionStatus {
        *self.status.read().unwrap()
    }

    /// Inspect current operational health.
    pub fn health(&self) -> RegionHealth {
        *self.health.read().unwrap()
    }

    /// Checks if the worker has stopped.
    pub fn is_stopped(&self) -> bool {
        self.status() == RegionStatus::Stopped
    }

    /// Checks if shutdown has been signaled to this worker.
    pub fn is_shutdown_requested(&self) -> bool {
        self.is_shutdown.load(Ordering::Relaxed)
    }

    /// Returns all unacknowledged durable loot intents extracted during shutdown.
    pub fn drained_intents(&self) -> Vec<DurableLootIntent> {
        self.drained_intents.read().unwrap().clone()
    }
}

/// Dedicated region worker executing deterministic simulation ticks on an OS thread.
pub struct RegionWorker {
    region_id: RegionId,
    epoch: u64,
    status: Arc<RwLock<RegionStatus>>,
    health: Arc<RwLock<RegionHealth>>,
    is_shutdown: Arc<AtomicBool>,
    engine: SimulationEngine,
    control_rx: Receiver<ControlMessage>,
    ack_rx: Receiver<PersistenceAck>,
    command_rx: Receiver<CommandEnvelope>,
    event_tx: Option<Sender<TickOutput>>,
    hooks: Box<dyn crate::hook::SimulationPhaseHooks>,
    tick_interval: Duration,
    lag_threshold: Duration,
    test_tick_delay: Duration,
    drain_tx: Option<oneshot::Sender<Vec<DurableLootIntent>>>,
    drained_intents: Arc<RwLock<Vec<DurableLootIntent>>>,
}

impl RegionWorker {
    /// Creates a new RegionWorker and its corresponding RegionHandle.
    #[must_use]
    pub fn new(
        region_id: RegionId,
        epoch: u64,
        map_grid: MapGrid,
        seed: u64,
        event_tx: Option<Sender<TickOutput>>,
    ) -> (Self, RegionHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (ack_tx, ack_rx) = mpsc::channel(PERSISTENCE_ACK_CAPACITY);
        let (control_tx, control_rx) = mpsc::channel(CONTROL_CHANNEL_CAPACITY);

        let status = Arc::new(RwLock::new(RegionStatus::Loading));
        let health = Arc::new(RwLock::new(RegionHealth::Healthy));
        let is_shutdown = Arc::new(AtomicBool::new(false));

        let width = map_grid.width_tiles();
        let height = map_grid.height_tiles();
        let mut world = RegionWorld::new(region_id, epoch, width, height);
        world.grid = map_grid;
        let engine = SimulationEngine::new(world, seed);

        let drained_intents = Arc::new(RwLock::new(Vec::new()));

        let handle = RegionHandle {
            region_id,
            epoch,
            cmd_tx,
            ack_tx,
            control_tx,
            status: Arc::clone(&status),
            health: Arc::clone(&health),
            is_shutdown: Arc::clone(&is_shutdown),
            drained_intents: Arc::clone(&drained_intents),
        };

        let worker = Self {
            region_id,
            epoch,
            status,
            health,
            is_shutdown,
            engine,
            control_rx,
            ack_rx,
            command_rx: cmd_rx,
            event_tx,
            hooks: Box::new(crate::hook::NoopSimulationHooks),
            tick_interval: Duration::from_millis(DEFAULT_TICK_INTERVAL_MS),
            lag_threshold: Duration::from_millis(LAG_THRESHOLD_MS),
            test_tick_delay: Duration::ZERO,
            drain_tx: None,
            drained_intents,
        };

        (worker, handle)
    }

    /// Spawns the worker thread and begins execution, transitioning to Active.
    pub fn spawn_thread(mut self) -> JoinHandle<()> {
        let thread_name = format!("region-{}", self.region_id.0);
        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                self.run();
            })
            .expect("failed to spawn region worker thread")
    }

    /// Direct access to inner simulation engine (for testing / initial population).
    pub fn engine_mut(&mut self) -> &mut SimulationEngine {
        &mut self.engine
    }

    /// Sets simulation phase hooks (e.g. integrated AI, economy, and dynamic event systems).
    pub fn set_hooks(&mut self, hooks: Box<dyn crate::hook::SimulationPhaseHooks>) {
        self.hooks = hooks;
    }

    /// Sets custom tick interval (useful for faster deterministic testing).
    pub fn set_tick_interval(&mut self, interval: Duration) {
        self.tick_interval = interval;
    }

    /// Sets custom lag threshold duration.
    pub fn set_lag_threshold(&mut self, threshold: Duration) {
        self.lag_threshold = threshold;
    }

    /// Sets an artificial tick delay (used to test lag detection and catch-up recovery).
    pub fn set_test_tick_delay(&mut self, delay: Duration) {
        self.test_tick_delay = delay;
    }

    /// Main loop executed on the dedicated worker OS thread.
    pub fn run(&mut self) {
        // 1. Transition Loading -> Active
        {
            let mut st = self.status.write().unwrap();
            if *st == RegionStatus::Loading {
                *st = RegionStatus::Active;
            }
        }

        let mut next_tick_instant = Instant::now();
        let mut consecutive_lagged_ticks: u32 = 0;
        let mut last_tick_duration_ms: u64 = 0;

        while !self.is_shutdown.load(Ordering::Relaxed) {
            // A. Process Control Messages (highest priority)
            if self.process_control_messages(&mut last_tick_duration_ms, consecutive_lagged_ticks) {
                // Shutdown requested
                break;
            }

            let current_status = *self.status.read().unwrap();
            if current_status == RegionStatus::Paused {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if current_status == RegionStatus::Stopped {
                break;
            }

            // B. Process Persistence Acks (high priority)
            self.process_persistence_acks();

            // C. Drain Player Commands from bounded ingress queue
            self.drain_ingress_commands();

            // D. Execute Tick
            let step_start = Instant::now();
            if self.test_tick_delay > Duration::ZERO {
                std::thread::sleep(self.test_tick_delay);
            }
            match self.engine.step_with_hooks(&mut *self.hooks) {
                Ok(output) => {
                    let step_duration = step_start.elapsed();
                    let step_duration_ms = step_duration.as_millis() as u64;
                    last_tick_duration_ms = step_duration_ms;

                    // Lag detection and health status management
                    if step_duration > self.lag_threshold {
                        consecutive_lagged_ticks = consecutive_lagged_ticks.saturating_add(1);
                        if consecutive_lagged_ticks >= 2 {
                            *self.health.write().unwrap() = RegionHealth::Degraded;
                        }
                    } else {
                        if consecutive_lagged_ticks > 0 {
                            consecutive_lagged_ticks = consecutive_lagged_ticks.saturating_sub(1);
                        }
                        if consecutive_lagged_ticks == 0 {
                            let mut h = self.health.write().unwrap();
                            if *h == RegionHealth::Degraded {
                                *h = RegionHealth::Healthy;
                            }
                        }
                    }

                    // Optional event emission / replication stream
                    if let Some(ref tx) = self.event_tx {
                        let _ = tx.try_send(output);
                    }
                }
                Err(err) => {
                    tracing::error!(region_id = %self.region_id.0, error = %err, "region simulation tick failed");
                    *self.health.write().unwrap() = RegionHealth::Failed;
                }
            }

            // E. Drift compensation and pacing
            next_tick_instant += self.tick_interval;
            let now = Instant::now();
            if now < next_tick_instant {
                std::thread::sleep(next_tick_instant - now);
            } else {
                // Lag accumulated
                let lag = now.duration_since(next_tick_instant);
                if lag > Duration::from_millis(MAX_ACCUMULATED_DRIFT_MS) {
                    // Severe drift: reset schedule to avoid death-spiral catchup bursts
                    next_tick_instant = now;
                }
            }
        }

        // F. Shutdown Drain Phase
        self.drain_shutdown();
    }

    /// Process incoming control messages without blocking.
    /// Returns true if Shutdown was received.
    fn process_control_messages(
        &mut self,
        last_tick_duration_ms: &mut u64,
        consecutive_lagged_ticks: u32,
    ) -> bool {
        while let Ok(msg) = self.control_rx.try_recv() {
            match msg {
                ControlMessage::Pause => {
                    *self.status.write().unwrap() = RegionStatus::Paused;
                }
                ControlMessage::Resume => {
                    *self.status.write().unwrap() = RegionStatus::Active;
                }
                ControlMessage::SetMode(mode) => {
                    let new_st = match mode {
                        SimulationMode::Active => RegionStatus::Active,
                        SimulationMode::Background => RegionStatus::Background,
                        SimulationMode::Paused => RegionStatus::Paused,
                    };
                    *self.status.write().unwrap() = new_st;
                }
                ControlMessage::Shutdown => {
                    return true;
                }
                ControlMessage::ShutdownDrain(resp_tx) => {
                    self.drain_tx = Some(resp_tx);
                    return true;
                }
                ControlMessage::DrainIntents(resp_tx) => {
                    let intents = self.engine.drain_pending_loot_intents();
                    let _ = resp_tx.send(intents);
                }
                ControlMessage::SetTestTickDelay(delay) => {
                    self.test_tick_delay = delay;
                }
                ControlMessage::InspectStatus(resp_tx) => {
                    let status = RegionWorkerStatus {
                        region_id: self.region_id,
                        epoch: self.epoch,
                        current_tick: self.engine.world.meta.tick,
                        status: *self.status.read().unwrap(),
                        health: *self.health.read().unwrap(),
                        last_tick_duration_ms: *last_tick_duration_ms,
                        consecutive_lagged_ticks,
                    };
                    let _ = resp_tx.send(status);
                }
            }
        }
        false
    }

    /// Processes persistence acknowledgments.
    fn process_persistence_acks(&mut self) {
        while let Ok(ack) = self.ack_rx.try_recv() {
            if ack.success {
                self.engine.acknowledge_loot_intent(ack.intent_id);
            }
        }
    }

    /// Drains up to channel capacity of ingress commands and submits to engine.
    fn drain_ingress_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            if cmd.ownership_epoch != self.epoch {
                tracing::warn!(
                    region_id = %self.region_id.0,
                    cmd_epoch = cmd.ownership_epoch,
                    worker_epoch = self.epoch,
                    "discarding command with stale ownership epoch"
                );
                continue;
            }
            let _ = self.engine.submit_envelope(cmd);
        }
    }

    /// Clean shutdown drain: drains remaining commands, updates state to Stopped.
    fn drain_shutdown(&mut self) {
        self.is_shutdown.store(true, Ordering::Relaxed);

        // Drain any remaining persistence acks
        self.process_persistence_acks();

        // Drain remaining commands (discarding or rejecting gracefully)
        while self.command_rx.try_recv().is_ok() {}

        // Drain pending intents without loss into shared outbox and optional drain_tx
        let pending = self.engine.drain_pending_loot_intents();
        if !pending.is_empty() {
            self.drained_intents
                .write()
                .unwrap()
                .extend(pending.clone());
        }
        if let Some(tx) = self.drain_tx.take() {
            let _ = tx.send(pending);
        }

        // Set Stopped status ONLY after all intents and acks are safely drained
        *self.status.write().unwrap() = RegionStatus::Stopped;
    }
}
