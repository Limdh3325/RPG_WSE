//! Region Coordinator and Command Router for multi-region simulation dispatch.
//!
//! Handles lifecycle management across multiple independent region workers,
//! ingress command routing, epoch validation, and backpressure handling.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use ecs_components::grid::MapGrid;
use game_domain::RegionId;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::command::CommandEnvelope;
use crate::engine::{DurableLootIntent, TickOutput};
use crate::worker::{
    ControlMessage, PersistenceAck, RegionHandle, RegionHealth, RegionStatus, RegionWorker,
    RegionWorkerStatus,
};

/// Errors encountered while routing commands to region workers.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouteError {
    #[error("unknown region: {0:?}")]
    UnknownRegion(RegionId),

    #[error("region {0:?} is stopped or unavailable")]
    RegionStopped(RegionId),

    #[error("server busy: region {region_id:?} command queue is saturated")]
    ServerBusy { region_id: RegionId },

    #[error("epoch mismatch for region {region_id:?}, expected {expected}")]
    EpochMismatch { region_id: RegionId, expected: u64 },
}

/// Errors encountered during region coordinator operations.
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("region already exists: {0:?}")]
    RegionAlreadyExists(RegionId),

    #[error("unknown region: {0:?}")]
    UnknownRegion(RegionId),

    #[error("failed to restore region state: {0}")]
    StateRestoreFailed(String),

    #[error("worker communication failed: {0}")]
    WorkerCommunication(String),
}

/// Type definition for epoch validation hooks.
pub type EpochValidator = Arc<dyn Fn(RegionId, u64) -> bool + Send + Sync>;

/// Central coordinator and router for multi-region simulation execution.
#[derive(Clone, Default)]
pub struct RegionCoordinator {
    regions: Arc<RwLock<HashMap<RegionId, RegionHandle>>>,
    threads: Arc<RwLock<HashMap<RegionId, JoinHandle<()>>>>,
    epoch_validator: Option<EpochValidator>,
    drained_intents: Arc<RwLock<HashMap<RegionId, Vec<DurableLootIntent>>>>,
}

impl RegionCoordinator {
    /// Creates a new empty RegionCoordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches an epoch validation hook.
    #[must_use]
    pub fn with_epoch_validator(mut self, validator: EpochValidator) -> Self {
        self.epoch_validator = Some(validator);
        self
    }

    /// Spawns and manages a new RegionWorker on a dedicated OS thread.
    pub fn spawn_region(
        &self,
        region_id: RegionId,
        epoch: u64,
        map_grid: MapGrid,
        seed: u64,
        event_tx: Option<Sender<TickOutput>>,
    ) -> Result<RegionHandle, CoordinatorError> {
        let mut regions = self.regions.write().unwrap();
        if regions.contains_key(&region_id) {
            return Err(CoordinatorError::RegionAlreadyExists(region_id));
        }

        let (worker, handle) = RegionWorker::new(region_id, epoch, map_grid, seed, event_tx);
        let thread_handle = worker.spawn_thread();

        regions.insert(region_id, handle.clone());
        self.threads
            .write()
            .unwrap()
            .insert(region_id, thread_handle);

        Ok(handle)
    }

    /// Spawns and manages a new RegionWorker configured with custom simulation hooks, tick interval, and optional restored state.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_region_configured(
        &self,
        region_id: RegionId,
        epoch: u64,
        map_grid: MapGrid,
        seed: u64,
        event_tx: Option<Sender<TickOutput>>,
        hooks: Option<Box<dyn crate::hook::SimulationPhaseHooks>>,
        tick_interval: Option<Duration>,
    ) -> Result<RegionHandle, CoordinatorError> {
        self.spawn_region_with_restored_state(
            region_id,
            epoch,
            map_grid,
            seed,
            event_tx,
            hooks,
            tick_interval,
            None,
        )
    }

    /// Spawns and manages a new RegionWorker configured with custom simulation hooks, tick interval, and restored canonical state.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_region_with_restored_state(
        &self,
        region_id: RegionId,
        epoch: u64,
        map_grid: MapGrid,
        seed: u64,
        event_tx: Option<Sender<TickOutput>>,
        hooks: Option<Box<dyn crate::hook::SimulationPhaseHooks>>,
        tick_interval: Option<Duration>,
        restored_state: Option<game_domain::RegionStateDto>,
    ) -> Result<RegionHandle, CoordinatorError> {
        let mut regions = self.regions.write().unwrap();
        if regions.contains_key(&region_id) {
            return Err(CoordinatorError::RegionAlreadyExists(region_id));
        }

        let (mut worker, handle) = RegionWorker::new(region_id, epoch, map_grid, seed, event_tx);
        if let Some(state) = restored_state {
            worker
                .restore_state(state)
                .map_err(|e| CoordinatorError::StateRestoreFailed(e.to_string()))?;
        }
        if let Some(h) = hooks {
            worker.set_hooks(h);
        }
        if let Some(interval) = tick_interval {
            worker.set_tick_interval(interval);
        }
        let thread_handle = worker.spawn_thread();

        regions.insert(region_id, handle.clone());
        self.threads
            .write()
            .unwrap()
            .insert(region_id, thread_handle);

        Ok(handle)
    }

    /// Registers a pre-existing RegionHandle directly.
    pub fn register_region(&self, handle: RegionHandle) -> Result<(), CoordinatorError> {
        let mut regions = self.regions.write().unwrap();
        if regions.contains_key(&handle.region_id) {
            return Err(CoordinatorError::RegionAlreadyExists(handle.region_id));
        }
        regions.insert(handle.region_id, handle);
        Ok(())
    }

    /// Registers an externally spawned region worker along with its join handle.
    pub fn register_region_with_thread(
        &self,
        handle: RegionHandle,
        thread: JoinHandle<()>,
    ) -> Result<(), CoordinatorError> {
        let region_id = handle.region_id;
        self.register_region(handle)?;
        self.threads.write().unwrap().insert(region_id, thread);
        Ok(())
    }

    /// Retrieves the handle for an active region.
    #[must_use]
    pub fn get_region(&self, region_id: RegionId) -> Option<RegionHandle> {
        self.regions.read().unwrap().get(&region_id).cloned()
    }

    /// Returns the number of registered regions.
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.read().unwrap().len()
    }

    /// Routes an incoming player command envelope to the appropriate region worker.
    ///
    /// Verifies region existence, validates epoch via hook, and applies non-blocking
    /// backpressure with `ServerBusy` when the command queue is saturated.
    pub fn route_command(&self, envelope: CommandEnvelope) -> Result<(), RouteError> {
        let region_id = envelope.region_id;
        let handle = {
            let regions = self.regions.read().unwrap();
            regions
                .get(&region_id)
                .cloned()
                .ok_or(RouteError::UnknownRegion(region_id))?
        };

        // 1. Check if region worker is stopped
        if handle.is_stopped() {
            return Err(RouteError::RegionStopped(region_id));
        }

        // 2. Validate Command Epoch matches Region Worker Epoch
        if envelope.ownership_epoch != handle.epoch {
            return Err(RouteError::EpochMismatch {
                region_id,
                expected: handle.epoch,
            });
        }

        // 3. Validate External Epoch Validator hook with command's ownership epoch
        if self
            .epoch_validator
            .as_ref()
            .is_some_and(|validator| !validator(region_id, envelope.ownership_epoch))
        {
            return Err(RouteError::EpochMismatch {
                region_id,
                expected: handle.epoch,
            });
        }

        // 4. Non-blocking backpressure send
        match handle.cmd_tx.try_send(envelope) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(RouteError::ServerBusy { region_id }),
            Err(TrySendError::Closed(_)) => Err(RouteError::RegionStopped(region_id)),
        }
    }

    /// Routes a persistence acknowledgment to the region worker.
    pub fn route_ack(
        &self,
        region_id: RegionId,
        ack: PersistenceAck,
    ) -> Result<(), CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        handle
            .ack_tx
            .try_send(ack)
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))
    }

    /// Pauses simulation ticks for a specific region.
    pub fn pause_region(&self, region_id: RegionId) -> Result<(), CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        handle
            .control_tx
            .try_send(ControlMessage::Pause)
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))
    }

    /// Resumes simulation ticks for a paused region.
    pub fn resume_region(&self, region_id: RegionId) -> Result<(), CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        handle
            .control_tx
            .try_send(ControlMessage::Resume)
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))
    }

    /// Stops a region worker, ensuring all unacknowledged durable loot intents are drained without loss.
    pub fn stop_region(
        &self,
        region_id: RegionId,
    ) -> Result<Vec<DurableLootIntent>, CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        // 1. Signal shutdown flag so worker exits loop even if control channel is saturated
        handle
            .is_shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // 2. Deliver Shutdown message, retrying briefly if buffer is currently full
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(500) {
            match handle.control_tx.try_send(ControlMessage::Shutdown) {
                Ok(()) => break,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }

        if let Some(join_handle) = self.threads.write().unwrap().remove(&region_id) {
            let _ = join_handle.join();
        } else {
            let start = std::time::Instant::now();
            while !handle.is_stopped() && start.elapsed() < std::time::Duration::from_secs(2) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        let drained = handle.drained_intents();
        self.drained_intents
            .write()
            .unwrap()
            .entry(region_id)
            .or_default()
            .extend(drained.clone());

        Ok(drained)
    }

    /// Stops all registered regions and collects all unacknowledged durable loot intents across regions.
    pub fn stop_all(&self) -> HashMap<RegionId, Vec<DurableLootIntent>> {
        let region_ids: Vec<RegionId> = {
            let regions = self.regions.read().unwrap();
            regions.keys().copied().collect()
        };

        let mut results = HashMap::new();
        for id in region_ids {
            if let Ok(drained) = self.stop_region(id) {
                results.insert(id, drained);
            }
        }
        results
    }

    /// Inspects the current status of a region asynchronously.
    pub async fn inspect_status(
        &self,
        region_id: RegionId,
    ) -> Result<RegionWorkerStatus, CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        let (tx, rx) = tokio::sync::oneshot::channel();
        handle
            .control_tx
            .try_send(ControlMessage::InspectStatus(tx))
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))?;

        rx.await
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))
    }

    /// Inspects the health of a region synchronously.
    pub fn get_health(&self, region_id: RegionId) -> Option<RegionHealth> {
        self.get_region(region_id).map(|h| h.health())
    }

    /// Inspects the status of a region synchronously.
    pub fn get_status(&self, region_id: RegionId) -> Option<RegionStatus> {
        self.get_region(region_id).map(|h| h.status())
    }

    /// Stops a region worker, gracefully draining all pending durable loot intents before termination.
    pub async fn stop_region_drain(
        &self,
        region_id: RegionId,
    ) -> Result<Vec<DurableLootIntent>, CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        handle
            .control_tx
            .send(ControlMessage::ShutdownDrain(resp_tx))
            .await
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))?;

        let drained_intents = resp_rx
            .await
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))?;

        if let Some(join_handle) = self.threads.write().unwrap().remove(&region_id) {
            let _ = join_handle.join();
        }

        self.drained_intents
            .write()
            .unwrap()
            .entry(region_id)
            .or_default()
            .extend(drained_intents.clone());

        Ok(drained_intents)
    }

    /// Drains pending durable loot intents from an active region without shutting it down.
    pub async fn drain_region_intents(
        &self,
        region_id: RegionId,
    ) -> Result<Vec<DurableLootIntent>, CoordinatorError> {
        let handle = self
            .get_region(region_id)
            .ok_or(CoordinatorError::UnknownRegion(region_id))?;

        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        handle
            .control_tx
            .try_send(ControlMessage::DrainIntents(resp_tx))
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))?;

        resp_rx
            .await
            .map_err(|e| CoordinatorError::WorkerCommunication(e.to_string()))
    }

    /// Returns any unacknowledged durable loot intents saved from stopped regions.
    pub fn get_saved_drained_intents(&self, region_id: RegionId) -> Vec<DurableLootIntent> {
        self.drained_intents
            .read()
            .unwrap()
            .get(&region_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Drains all accumulated saved intents across all stopped regions for persistence outbox handoff.
    pub fn drain_all_saved_intents(&self) -> HashMap<RegionId, Vec<DurableLootIntent>> {
        std::mem::take(&mut *self.drained_intents.write().unwrap())
    }
}
