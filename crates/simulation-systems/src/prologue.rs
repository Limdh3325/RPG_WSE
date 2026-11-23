//! Prologue progression state machine, resilience fallbacks, and idempotent world transfer.
//!
//! Supports TASK-012B and TEST-PRO-001 through TEST-PRO-006:
//! - P0 to P5 onboarding progression
//! - Disconnection recovery and durable checkpointing
//! - Fallback pathways for pathing failures (5s timeout notice) and NPC deaths
//! - Idempotent transfer from private instance regions to shared settlement worlds

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrologueError {
    #[error("Prologue has not started")]
    NotStarted,
    #[error("Prologue is already completed")]
    AlreadyCompleted,
    #[error("Invalid step transition from {from:?} to {to:?}")]
    InvalidStepTransition {
        from: PrologueStep,
        to: PrologueStep,
    },
    #[error("Action does not match current step {expected:?}")]
    ActionMismatch { expected: PrologueStep },
    #[error("Transfer not ready: current state is {0:?}")]
    TransferNotReady(String),
    #[error("Character {0} not found")]
    CharacterNotFound(Uuid),
    #[error("Transfer conflict: character {0} already in-flight")]
    TransferConflict(Uuid),
    #[error("Persistence error: {0}")]
    PersistenceError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrologueStep {
    /// P0: Meet Elder Elda and learn basic dialogue/controls.
    P0VillageEntry,
    /// P1: Gather medicinal herbs / timber (3 units).
    P1ResourceGathering,
    /// P2: Travel towards the frontier outpost.
    P2OutpostTravel,
    /// P3: Defeat goblin scout and rescue patrol soldier Kael.
    P3RescueCombat,
    /// P4: Pack migration wagon supplies.
    P4SupplyMigration,
    /// P5: Reach shared settlement boundary gate and trigger transfer.
    P5SharedSettlement,
}

impl PrologueStep {
    pub fn description(&self) -> &'static str {
        match self {
            Self::P0VillageEntry => "개척촌 입장: 촌장 엘다와 대화하여 상황 파악하기",
            Self::P1ResourceGathering => "자원 획득: 약초와 자재 3개 채집하기",
            Self::P2OutpostTravel => "국경 이동: 국경 전초기지로 이동하기",
            Self::P3RescueCombat => "전투/구조: 고블린을 격퇴하고 정찰병 카엘 구출하기",
            Self::P4SupplyMigration => "보급 준비: 이주 마차에 보급품 싣기",
            Self::P5SharedSettlement => "공유 세계 진입: 개척 연맹 정착지 관문으로 진입하기",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrologueAction {
    TalkToNpc { npc_name: String },
    GatherResource { resource_type: String, amount: u32 },
    ReachWaypoint { x: i32, y: i32 },
    DefeatEncounter { target_id: String },
    PackSupplies { supply_id: String },
    EnterSettlementGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrologueRewardReceipt {
    pub receipt_id: Uuid,
    pub sku: String,
    pub quantity: u32,
    pub granted_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Preparing,
    Pending,
    Committed,
    Reverted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrologueTransferRecord {
    pub transfer_id: Uuid,
    pub character_id: Uuid,
    pub source_region_id: Uuid,
    pub target_region_id: Uuid,
    pub status: TransferStatus,
    pub rewards: Vec<PrologueRewardReceipt>,
    pub attempts: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrologueState {
    NotStarted,
    InProgress {
        step: PrologueStep,
        progress: u32,
        target_count: u32,
        fallback_active: bool,
        last_failure_reason: Option<String>,
        last_updated_ms: i64,
    },
    ReadyToTransfer {
        transfer_id: Uuid,
        rewards: Vec<PrologueRewardReceipt>,
        ready_at_ms: i64,
    },
    Transferring {
        transfer_id: Uuid,
        started_at_ms: i64,
    },
    Completed {
        completed_at_ms: i64,
        target_region_id: Uuid,
        transfer_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterPrologueSession {
    pub character_id: Uuid,
    pub private_region_id: Uuid,
    pub state: PrologueState,
    pub rewards_accumulated: Vec<PrologueRewardReceipt>,
    pub fallback_triggered_count: u32,
}

#[derive(Debug, Default)]
pub struct TransferRegistry {
    pub transfers: HashMap<Uuid, PrologueTransferRecord>,
    pub completed_character_transfers: HashMap<Uuid, Uuid>,
    pub issued_receipt_ids: HashSet<Uuid>,
}

impl TransferRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Idempotently executes or retries a transfer.
    /// If the transfer has already committed, returns existing record without duplicate grants.
    pub fn execute_transfer(
        &mut self,
        transfer_id: Uuid,
        character_id: Uuid,
        source_region_id: Uuid,
        target_region_id: Uuid,
        rewards: Vec<PrologueRewardReceipt>,
        now_ms: i64,
    ) -> Result<PrologueTransferRecord, PrologueError> {
        // If already completed for this character, verify transfer_id match
        if let Some(record) = self
            .completed_character_transfers
            .get(&character_id)
            .and_then(|tid| self.transfers.get_mut(tid))
        {
            record.attempts += 1;
            record.updated_at_ms = now_ms;
            return Ok(record.clone());
        }

        // If this transfer record exists
        if let Some(record) = self.transfers.get_mut(&transfer_id) {
            record.attempts += 1;
            record.updated_at_ms = now_ms;
            if record.status == TransferStatus::Committed {
                return Ok(record.clone());
            }
            // Mark committed
            record.status = TransferStatus::Committed;
            self.completed_character_transfers
                .insert(character_id, transfer_id);
            return Ok(record.clone());
        }

        // Register receipts safely
        let mut deduplicated_rewards = Vec::new();
        for r in rewards {
            if self.issued_receipt_ids.insert(r.receipt_id) {
                deduplicated_rewards.push(r);
            }
        }

        let record = PrologueTransferRecord {
            transfer_id,
            character_id,
            source_region_id,
            target_region_id,
            status: TransferStatus::Committed,
            rewards: deduplicated_rewards,
            attempts: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };

        self.transfers.insert(transfer_id, record.clone());
        self.completed_character_transfers
            .insert(character_id, transfer_id);
        Ok(record)
    }
}

/// Durable storage interface for prologue sessions and transfers (TASK-012B).
pub trait PrologueStorage: Send + Sync {
    fn save_session(&self, session: &CharacterPrologueSession) -> Result<(), PrologueError>;
    fn load_session(
        &self,
        character_id: &Uuid,
    ) -> Result<Option<CharacterPrologueSession>, PrologueError>;
    fn save_transfer(&self, record: &PrologueTransferRecord) -> Result<(), PrologueError>;
    fn load_transfer(
        &self,
        transfer_id: &Uuid,
    ) -> Result<Option<PrologueTransferRecord>, PrologueError>;
    fn load_all_sessions(&self) -> Result<Vec<CharacterPrologueSession>, PrologueError>;
    fn load_all_transfers(&self) -> Result<Vec<PrologueTransferRecord>, PrologueError>;
}

/// File-based atomic JSON persistence adapter for prologue durability.
pub struct FilePrologueStorage {
    base_dir: PathBuf,
}

impl FilePrologueStorage {
    pub fn new<P: AsRef<Path>>(base_dir: P) -> std::io::Result<Self> {
        let path = base_dir.as_ref().to_path_buf();
        fs::create_dir_all(path.join("sessions"))?;
        fs::create_dir_all(path.join("transfers"))?;
        Ok(Self { base_dir: path })
    }
}

impl PrologueStorage for FilePrologueStorage {
    fn save_session(&self, session: &CharacterPrologueSession) -> Result<(), PrologueError> {
        let file_path = self
            .base_dir
            .join("sessions")
            .join(format!("{}.json", session.character_id));
        let tmp_path = file_path.with_extension("tmp");
        let json = serde_json::to_string_pretty(session)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        fs::write(&tmp_path, json).map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        fs::rename(tmp_path, file_path)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        Ok(())
    }

    fn load_session(
        &self,
        character_id: &Uuid,
    ) -> Result<Option<CharacterPrologueSession>, PrologueError> {
        let file_path = self
            .base_dir
            .join("sessions")
            .join(format!("{}.json", character_id));
        if !file_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file_path)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        let session = serde_json::from_str(&content)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        Ok(Some(session))
    }

    fn save_transfer(&self, record: &PrologueTransferRecord) -> Result<(), PrologueError> {
        let file_path = self
            .base_dir
            .join("transfers")
            .join(format!("{}.json", record.transfer_id));
        let tmp_path = file_path.with_extension("tmp");
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        fs::write(&tmp_path, json).map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        fs::rename(tmp_path, file_path)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        Ok(())
    }

    fn load_transfer(
        &self,
        transfer_id: &Uuid,
    ) -> Result<Option<PrologueTransferRecord>, PrologueError> {
        let file_path = self
            .base_dir
            .join("transfers")
            .join(format!("{}.json", transfer_id));
        if !file_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file_path)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        let record = serde_json::from_str(&content)
            .map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
        Ok(Some(record))
    }

    fn load_all_sessions(&self) -> Result<Vec<CharacterPrologueSession>, PrologueError> {
        let dir = self.base_dir.join("sessions");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut list = Vec::new();
        for entry in
            fs::read_dir(dir).map_err(|e| PrologueError::PersistenceError(e.to_string()))?
        {
            let entry = entry.map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
            let path = entry.path();
            if let Some(sess) = load_json_entry::<CharacterPrologueSession>(&path) {
                list.push(sess);
            }
        }
        Ok(list)
    }

    fn load_all_transfers(&self) -> Result<Vec<PrologueTransferRecord>, PrologueError> {
        let dir = self.base_dir.join("transfers");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut list = Vec::new();
        for entry in
            fs::read_dir(dir).map_err(|e| PrologueError::PersistenceError(e.to_string()))?
        {
            let entry = entry.map_err(|e| PrologueError::PersistenceError(e.to_string()))?;
            let path = entry.path();
            if let Some(rec) = load_json_entry::<PrologueTransferRecord>(&path) {
                list.push(rec);
            }
        }
        Ok(list)
    }
}

fn load_json_entry<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    if path.extension().and_then(|s| s.to_str()) != Some("json") {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub struct PrologueManager {
    sessions: HashMap<Uuid, CharacterPrologueSession>,
    pub transfer_registry: TransferRegistry,
    storage: Option<Arc<dyn PrologueStorage>>,
}

impl Default for PrologueManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PrologueManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            transfer_registry: TransferRegistry::new(),
            storage: None,
        }
    }

    pub fn try_new_with_storage(storage: Arc<dyn PrologueStorage>) -> Result<Self, PrologueError> {
        let mut sessions = HashMap::new();
        let mut transfer_registry = TransferRegistry::new();

        let loaded_sessions = storage.load_all_sessions()?;
        for s in loaded_sessions {
            sessions.insert(s.character_id, s);
        }

        let loaded_transfers = storage.load_all_transfers()?;
        for t in loaded_transfers {
            for r in &t.rewards {
                transfer_registry.issued_receipt_ids.insert(r.receipt_id);
            }
            if t.status == TransferStatus::Committed {
                transfer_registry
                    .completed_character_transfers
                    .insert(t.character_id, t.transfer_id);
            }
            transfer_registry.transfers.insert(t.transfer_id, t);
        }

        let mut manager = Self {
            sessions,
            transfer_registry,
            storage: Some(storage),
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        manager.reconcile_inflight_transfers(now_ms)?;

        Ok(manager)
    }

    /// Reconciles any in-flight transfers interrupted by server crash or restart.
    /// Moves verified in-flight records to Completed and updates durable state.
    pub fn reconcile_inflight_transfers(&mut self, now_ms: i64) -> Result<u32, PrologueError> {
        let mut reconciled = 0;
        let character_ids: Vec<Uuid> = self.sessions.keys().copied().collect();

        for cid in character_ids {
            let in_flight = self.sessions.get(&cid).and_then(|s| match s.state {
                PrologueState::Transferring { transfer_id, .. } => Some(transfer_id),
                _ => None,
            });

            let Some(tid) = in_flight else { continue };
            let record = self.transfer_registry.transfers.get(&tid);

            match record {
                Some(r) if r.status == TransferStatus::Committed => {
                    let target_region_id = r.target_region_id;
                    if let Some(session) = self.sessions.get_mut(&cid) {
                        session.state = PrologueState::Completed {
                            completed_at_ms: now_ms,
                            target_region_id,
                            transfer_id: tid,
                        };
                        if let Some(storage) = &self.storage {
                            storage.save_session(session)?;
                        }
                        reconciled += 1;
                    }
                }
                _ => {
                    // Transfer record was not committed before crash; rollback to ReadyToTransfer so player can retry transfer
                    if let Some(session) = self.sessions.get_mut(&cid) {
                        let rewards = session.rewards_accumulated.clone();
                        session.state = PrologueState::ReadyToTransfer {
                            transfer_id: tid,
                            rewards,
                            ready_at_ms: now_ms,
                        };
                        if let Some(storage) = &self.storage {
                            storage.save_session(session)?;
                        }
                        reconciled += 1;
                    }
                }
            }
        }

        Ok(reconciled)
    }

    pub fn new_with_storage(storage: Arc<dyn PrologueStorage>) -> Self {
        Self::try_new_with_storage(storage).expect("failed to load prologue storage state")
    }

    pub fn get_state(&self, character_id: &Uuid) -> PrologueState {
        self.sessions
            .get(character_id)
            .map(|s| s.state.clone())
            .unwrap_or(PrologueState::NotStarted)
    }

    pub fn is_completed(&self, character_id: &Uuid) -> bool {
        matches!(
            self.get_state(character_id),
            PrologueState::Completed { .. }
        ) || self
            .transfer_registry
            .completed_character_transfers
            .contains_key(character_id)
    }

    pub fn get_completed_target_region(&self, character_id: &Uuid) -> Option<Uuid> {
        if let PrologueState::Completed {
            target_region_id, ..
        } = self.get_state(character_id)
        {
            return Some(target_region_id);
        }
        if let Some(record) = self
            .transfer_registry
            .completed_character_transfers
            .get(character_id)
            .and_then(|tid| self.transfer_registry.transfers.get(tid))
        {
            return Some(record.target_region_id);
        }
        None
    }

    pub fn start_prologue(
        &mut self,
        character_id: Uuid,
        private_region_id: Uuid,
        now_ms: i64,
    ) -> Result<PrologueState, PrologueError> {
        if self.is_completed(&character_id) {
            return Err(PrologueError::AlreadyCompleted);
        }

        let initial_state = PrologueState::InProgress {
            step: PrologueStep::P0VillageEntry,
            progress: 0,
            target_count: 1,
            fallback_active: false,
            last_failure_reason: None,
            last_updated_ms: now_ms,
        };

        let session = CharacterPrologueSession {
            character_id,
            private_region_id,
            state: initial_state.clone(),
            rewards_accumulated: Vec::new(),
            fallback_triggered_count: 0,
        };

        if let Some(storage) = &self.storage {
            storage.save_session(&session)?;
        }
        self.sessions.insert(character_id, session);
        Ok(initial_state)
    }

    /// Advance step based on user gameplay action.
    pub fn advance_step(
        &mut self,
        character_id: Uuid,
        action: PrologueAction,
        now_ms: i64,
    ) -> Result<PrologueState, PrologueError> {
        let session = self
            .sessions
            .get_mut(&character_id)
            .ok_or(PrologueError::CharacterNotFound(character_id))?;

        match &session.state {
            PrologueState::NotStarted => Err(PrologueError::NotStarted),
            PrologueState::Completed { .. } => Err(PrologueError::AlreadyCompleted),
            PrologueState::ReadyToTransfer { .. } | PrologueState::Transferring { .. } => {
                Err(PrologueError::InvalidStepTransition {
                    from: PrologueStep::P5SharedSettlement,
                    to: PrologueStep::P5SharedSettlement,
                })
            }
            PrologueState::InProgress {
                step,
                progress,
                target_count,
                ..
            } => {
                let current_step = *step;
                let current_progress = *progress;
                let current_target = *target_count;

                let res = match (current_step, &action) {
                    (PrologueStep::P0VillageEntry, PrologueAction::TalkToNpc { .. }) => {
                        // Advance to P1
                        let next_state = PrologueState::InProgress {
                            step: PrologueStep::P1ResourceGathering,
                            progress: 0,
                            target_count: 3,
                            fallback_active: false,
                            last_failure_reason: None,
                            last_updated_ms: now_ms,
                        };
                        session.state = next_state.clone();
                        Ok(next_state)
                    }
                    (
                        PrologueStep::P1ResourceGathering,
                        PrologueAction::GatherResource { amount, .. },
                    ) => {
                        let new_prog = (current_progress + amount).min(current_target);
                        if new_prog >= current_target {
                            // Grant reward receipt for P1
                            session.rewards_accumulated.push(PrologueRewardReceipt {
                                receipt_id: Uuid::new_v4(),
                                sku: "item.starter_potion".to_string(),
                                quantity: 5,
                                granted_at_ms: now_ms,
                            });
                            let next_state = PrologueState::InProgress {
                                step: PrologueStep::P2OutpostTravel,
                                progress: 0,
                                target_count: 1,
                                fallback_active: false,
                                last_failure_reason: None,
                                last_updated_ms: now_ms,
                            };
                            session.state = next_state.clone();
                            Ok(next_state)
                        } else {
                            let next_state = PrologueState::InProgress {
                                step: PrologueStep::P1ResourceGathering,
                                progress: new_prog,
                                target_count: current_target,
                                fallback_active: false,
                                last_failure_reason: None,
                                last_updated_ms: now_ms,
                            };
                            session.state = next_state.clone();
                            Ok(next_state)
                        }
                    }
                    (PrologueStep::P2OutpostTravel, PrologueAction::ReachWaypoint { .. }) => {
                        let next_state = PrologueState::InProgress {
                            step: PrologueStep::P3RescueCombat,
                            progress: 0,
                            target_count: 1,
                            fallback_active: false,
                            last_failure_reason: None,
                            last_updated_ms: now_ms,
                        };
                        session.state = next_state.clone();
                        Ok(next_state)
                    }
                    (PrologueStep::P3RescueCombat, PrologueAction::DefeatEncounter { .. }) => {
                        // Reward combat gear
                        session.rewards_accumulated.push(PrologueRewardReceipt {
                            receipt_id: Uuid::new_v4(),
                            sku: "equipment.militia_blade".to_string(),
                            quantity: 1,
                            granted_at_ms: now_ms,
                        });
                        let next_state = PrologueState::InProgress {
                            step: PrologueStep::P4SupplyMigration,
                            progress: 0,
                            target_count: 1,
                            fallback_active: false,
                            last_failure_reason: None,
                            last_updated_ms: now_ms,
                        };
                        session.state = next_state.clone();
                        Ok(next_state)
                    }
                    (PrologueStep::P4SupplyMigration, PrologueAction::PackSupplies { .. }) => {
                        let next_state = PrologueState::InProgress {
                            step: PrologueStep::P5SharedSettlement,
                            progress: 0,
                            target_count: 1,
                            fallback_active: false,
                            last_failure_reason: None,
                            last_updated_ms: now_ms,
                        };
                        session.state = next_state.clone();
                        Ok(next_state)
                    }
                    (PrologueStep::P5SharedSettlement, PrologueAction::EnterSettlementGate) => {
                        // Transfer ready!
                        let transfer_id = Uuid::new_v4();
                        let next_state = PrologueState::ReadyToTransfer {
                            transfer_id,
                            rewards: session.rewards_accumulated.clone(),
                            ready_at_ms: now_ms,
                        };
                        session.state = next_state.clone();
                        Ok(next_state)
                    }
                    _ => Err(PrologueError::ActionMismatch {
                        expected: current_step,
                    }),
                };

                if let Some(storage) = self.storage.as_ref().filter(|_| res.is_ok()) {
                    storage.save_session(session)?;
                }
                res
            }
        }
    }

    /// Handles pathing failure or blocked conditions. Within 5s UI target,
    /// marks fallback active and allows either rerouting or bypass.
    pub fn report_movement_failure(
        &mut self,
        character_id: Uuid,
        reason: &str,
        now_ms: i64,
    ) -> Result<PrologueState, PrologueError> {
        let session = self
            .sessions
            .get_mut(&character_id)
            .ok_or(PrologueError::CharacterNotFound(character_id))?;

        if let PrologueState::InProgress {
            step,
            progress,
            target_count,
            ..
        } = session.state
        {
            session.fallback_triggered_count += 1;
            let updated = PrologueState::InProgress {
                step,
                progress,
                target_count,
                fallback_active: true,
                last_failure_reason: Some(reason.to_string()),
                last_updated_ms: now_ms,
            };
            session.state = updated.clone();
            if let Some(storage) = &self.storage {
                storage.save_session(session)?;
            }
            Ok(updated)
        } else {
            Ok(session.state.clone())
        }
    }

    /// Handles NPC casualty or combat failure (TEST-PRO-006).
    /// Provides emergency alternative note/clue so progression is never permanently blocked.
    pub fn trigger_npc_casualty_fallback(
        &mut self,
        character_id: Uuid,
        npc_name: &str,
        now_ms: i64,
    ) -> Result<PrologueState, PrologueError> {
        let session = self
            .sessions
            .get_mut(&character_id)
            .ok_or(PrologueError::CharacterNotFound(character_id))?;

        if let PrologueState::InProgress { step, .. } = session.state {
            session.fallback_triggered_count += 1;
            // Advance with emergency bypass
            let next_step = match step {
                PrologueStep::P0VillageEntry => PrologueStep::P1ResourceGathering,
                PrologueStep::P1ResourceGathering => PrologueStep::P2OutpostTravel,
                PrologueStep::P2OutpostTravel => PrologueStep::P3RescueCombat,
                PrologueStep::P3RescueCombat => {
                    // Even if soldier falls, player loots scout badge and moves on
                    session.rewards_accumulated.push(PrologueRewardReceipt {
                        receipt_id: Uuid::new_v4(),
                        sku: "item.scout_badge".to_string(),
                        quantity: 1,
                        granted_at_ms: now_ms,
                    });
                    PrologueStep::P4SupplyMigration
                }
                PrologueStep::P4SupplyMigration => PrologueStep::P5SharedSettlement,
                PrologueStep::P5SharedSettlement => {
                    let transfer_id = Uuid::new_v4();
                    let state = PrologueState::ReadyToTransfer {
                        transfer_id,
                        rewards: session.rewards_accumulated.clone(),
                        ready_at_ms: now_ms,
                    };
                    session.state = state.clone();
                    if let Some(storage) = &self.storage {
                        storage.save_session(session)?;
                    }
                    return Ok(state);
                }
            };

            let updated = PrologueState::InProgress {
                step: next_step,
                progress: 0,
                target_count: 1,
                fallback_active: true,
                last_failure_reason: Some(format!("대체 경로 활성화: {} 유품 수습", npc_name)),
                last_updated_ms: now_ms,
            };
            session.state = updated.clone();
            if let Some(storage) = &self.storage {
                storage.save_session(session)?;
            }
            Ok(updated)
        } else {
            Ok(session.state.clone())
        }
    }

    /// Initiates and completes world transfer into target shared settlement region.
    pub fn transfer_to_shared_world(
        &mut self,
        character_id: Uuid,
        target_region_id: Uuid,
        now_ms: i64,
    ) -> Result<PrologueTransferRecord, PrologueError> {
        let (transfer_id, source_region_id, rewards) = {
            let session = self
                .sessions
                .get_mut(&character_id)
                .ok_or(PrologueError::CharacterNotFound(character_id))?;

            match &session.state {
                PrologueState::ReadyToTransfer {
                    transfer_id,
                    rewards,
                    ..
                } => (*transfer_id, session.private_region_id, rewards.clone()),
                PrologueState::Transferring { transfer_id, .. } => (
                    *transfer_id,
                    session.private_region_id,
                    session.rewards_accumulated.clone(),
                ),
                PrologueState::Completed {
                    transfer_id,
                    target_region_id: cur_target,
                    completed_at_ms,
                } => {
                    return Ok(PrologueTransferRecord {
                        transfer_id: *transfer_id,
                        character_id,
                        source_region_id: session.private_region_id,
                        target_region_id: *cur_target,
                        status: TransferStatus::Committed,
                        rewards: session.rewards_accumulated.clone(),
                        attempts: 1,
                        created_at_ms: *completed_at_ms,
                        updated_at_ms: now_ms,
                    });
                }
                other => {
                    return Err(PrologueError::TransferNotReady(format!("{:?}", other)));
                }
            }
        };

        // 1. Enter Transferring state (lock source region and record in-flight)
        if let Some(session) = self.sessions.get_mut(&character_id) {
            session.state = PrologueState::Transferring {
                transfer_id,
                started_at_ms: now_ms,
            };
            if let Some(storage) = &self.storage {
                storage.save_session(session)?;
            }
        }

        // 2. Execute transfer idempotently
        let record = self.transfer_registry.execute_transfer(
            transfer_id,
            character_id,
            source_region_id,
            target_region_id,
            rewards,
            now_ms,
        )?;

        // 3. Update session to Completed
        if let Some(session) = self.sessions.get_mut(&character_id) {
            session.state = PrologueState::Completed {
                completed_at_ms: now_ms,
                target_region_id,
                transfer_id,
            };
            if let Some(storage) = &self.storage {
                storage.save_transfer(&record)?;
                storage.save_session(session)?;
            }
        }

        Ok(record)
    }

    /// Restore or update session state for durable disconnection recovery.
    pub fn restore_session(
        &mut self,
        session: CharacterPrologueSession,
    ) -> Result<(), PrologueError> {
        if let PrologueState::Completed {
            transfer_id,
            target_region_id,
            completed_at_ms,
        } = session.state
        {
            self.transfer_registry
                .completed_character_transfers
                .insert(session.character_id, transfer_id);
            self.transfer_registry
                .transfers
                .entry(transfer_id)
                .or_insert_with(|| PrologueTransferRecord {
                    transfer_id,
                    character_id: session.character_id,
                    source_region_id: session.private_region_id,
                    target_region_id,
                    status: TransferStatus::Committed,
                    rewards: session.rewards_accumulated.clone(),
                    attempts: 1,
                    created_at_ms: completed_at_ms,
                    updated_at_ms: completed_at_ms,
                });
        }
        if let Some(storage) = &self.storage {
            storage.save_session(&session)?;
        }
        self.sessions.insert(session.character_id, session);
        Ok(())
    }
}
