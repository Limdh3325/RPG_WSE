use game_domain::errors::DomainError;
use game_domain::id::{CommandId, EntityId, RegionId, SessionId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Supported command schema version.
pub const SUPPORTED_COMMAND_SCHEMA_VERSION: u32 = 1;

/// Lifecycle state machine for commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CommandStatus {
    /// Received by ingress gateway/buffer.
    Received,
    /// Passed schema validation, dedup, and admitted into execution queue.
    Accepted,
    /// Validation failed or rejected prior to execution.
    Rejected,
    /// Currently being executed in a simulation tick, pending final commit.
    PendingCommit,
    /// Successfully executed and committed.
    Succeeded,
    /// Execution failed during simulation.
    Failed,
}

impl CommandStatus {
    /// Returns true if transition from `self` to `next` is valid according to contract.
    #[must_use]
    pub fn can_transition_to(&self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Received, Self::Accepted | Self::Rejected)
                | (Self::Accepted, Self::PendingCommit | Self::Rejected)
                | (Self::PendingCommit, Self::Succeeded | Self::Failed)
        )
    }

    /// Transitions status to next state if allowed by state machine, otherwise returns error.
    pub fn transition_to(&mut self, next: Self) -> Result<(), DomainError> {
        if self.can_transition_to(next) {
            *self = next;
            Ok(())
        } else {
            Err(DomainError::InvalidCommandStateTransition {
                from: format!("{self:?}"),
                to: format!("{next:?}"),
            })
        }
    }
}

/// Commands accepted by simulation engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandPayload {
    Move { target_x: i64, target_y: i64 },
    SetVelocity { dx: i32, dy: i32 },
    Attack { target_id: EntityId },
    Purchase { item_id: u64, price: i64 },
    Custom { action: String, payload: Vec<u8> },
    Noop,
}

/// Full specification envelope for client commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub actor_id: EntityId,
    pub region_id: RegionId,
    pub session_id: SessionId,
    pub client_seq: u64,
    pub assigned_tick: u64,
    pub ingress_seq: u64,
    pub ownership_epoch: u64,
    pub schema_version: u32,
    pub payload: CommandPayload,
    pub payload_hash: [u8; 32],
}

impl CommandEnvelope {
    /// Constructs a new envelope with auto-calculated payload SHA-256 hash.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        command_id: CommandId,
        actor_id: EntityId,
        region_id: RegionId,
        session_id: SessionId,
        client_seq: u64,
        assigned_tick: u64,
        ingress_seq: u64,
        ownership_epoch: u64,
        payload: CommandPayload,
    ) -> Self {
        let payload_hash = Self::calculate_payload_hash(&payload);
        Self {
            command_id,
            actor_id,
            region_id,
            session_id,
            client_seq,
            assigned_tick,
            ingress_seq,
            ownership_epoch,
            schema_version: SUPPORTED_COMMAND_SCHEMA_VERSION,
            payload,
            payload_hash,
        }
    }

    /// Computes canonical SHA-256 hash of the command payload.
    #[must_use]
    pub fn calculate_payload_hash(payload: &CommandPayload) -> [u8; 32] {
        let serialized = serde_json::to_vec(payload).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }

    /// Validates schema version and payload hash integrity against payload content.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != SUPPORTED_COMMAND_SCHEMA_VERSION {
            return Err(DomainError::InvalidSchemaVersion {
                supported: SUPPORTED_COMMAND_SCHEMA_VERSION,
                received: self.schema_version,
            });
        }
        let computed = Self::calculate_payload_hash(&self.payload);
        if computed != self.payload_hash {
            return Err(DomainError::InvalidCommand {
                reason: format!(
                    "payload hash mismatch: computed {:?}, envelope {:?}",
                    computed, self.payload_hash
                ),
            });
        }
        Ok(())
    }

    /// Converts this envelope to an IngressCommand without truncating command_id.
    #[must_use]
    pub fn to_ingress(&self) -> IngressCommand {
        IngressCommand {
            command_id: self.command_id,
            entity_id: self.actor_id,
            assigned_tick: self.assigned_tick,
            ingress_seq: self.ingress_seq,
            payload: self.payload.clone(),
        }
    }
}

/// An ingress command assigned to be executed at a specific tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressCommand {
    pub command_id: CommandId,
    pub entity_id: EntityId,
    pub assigned_tick: u64,
    pub ingress_seq: u64,
    pub payload: CommandPayload,
}

impl IngressCommand {
    #[must_use]
    pub fn new(
        command_id: impl Into<CommandId>,
        entity_id: EntityId,
        assigned_tick: u64,
        ingress_seq: u64,
        payload: CommandPayload,
    ) -> Self {
        Self {
            command_id: command_id.into(),
            entity_id,
            assigned_tick,
            ingress_seq,
            payload,
        }
    }

    #[must_use]
    pub fn to_envelope(
        &self,
        region_id: RegionId,
        session_id: SessionId,
        client_seq: u64,
        ownership_epoch: u64,
    ) -> CommandEnvelope {
        CommandEnvelope::new(
            self.command_id,
            self.entity_id,
            region_id,
            session_id,
            client_seq,
            self.assigned_tick,
            self.ingress_seq,
            ownership_epoch,
            self.payload.clone(),
        )
    }
}

/// Outcome of transient command deduplication check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeduplicationOutcome {
    /// Command is new and registered.
    New,
    /// Command was already processed/scheduled with identical payload.
    Duplicate {
        original_tick: u64,
        status: CommandStatus,
    },
}

/// In-memory transient command deduplication buffer.
#[derive(Debug, Clone)]
pub struct TransientCommandDeduplicator {
    cache: BTreeMap<CommandId, ([u8; 32], u64, CommandStatus)>,
    capacity: usize,
}

impl TransientCommandDeduplicator {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: BTreeMap::new(),
            capacity: capacity.max(1),
        }
    }

    /// Checks command against deduplication buffer.
    /// Returns `DeduplicationOutcome::New` if accepted, `Duplicate` if duplicate identical command,
    /// or `Err(IdempotencyConflict)` if command_id was reused with differing payload.
    pub fn check_and_register(
        &mut self,
        envelope: &CommandEnvelope,
    ) -> Result<DeduplicationOutcome, DomainError> {
        // Enforce schema version and payload hash integrity
        envelope.validate()?;

        if let Some(&(existing_hash, original_tick, status)) = self.cache.get(&envelope.command_id)
        {
            if existing_hash != envelope.payload_hash {
                return Err(DomainError::IdempotencyConflict {
                    command_id: envelope.command_id.0,
                    existing_hash,
                    incoming_hash: envelope.payload_hash,
                });
            }
            return Ok(DeduplicationOutcome::Duplicate {
                original_tick,
                status,
            });
        }

        if self.cache.len() >= self.capacity && !self.cache.is_empty() {
            let first_key = *self.cache.keys().next().expect("cache not empty");
            self.cache.remove(&first_key);
        }

        self.cache.insert(
            envelope.command_id,
            (
                envelope.payload_hash,
                envelope.assigned_tick,
                CommandStatus::Accepted,
            ),
        );

        Ok(DeduplicationOutcome::New)
    }

    /// Updates status of an existing command in cache with state machine transition enforcement.
    pub fn update_status(
        &mut self,
        command_id: CommandId,
        new_status: CommandStatus,
    ) -> Result<(), DomainError> {
        if let Some((_, _, status)) = self.cache.get_mut(&command_id) {
            status.transition_to(new_status)
        } else {
            Err(DomainError::InvalidCommand {
                reason: format!("command {command_id} not found in deduplicator cache"),
            })
        }
    }

    /// Queries status of command if present.
    #[must_use]
    pub fn get_status(&self, command_id: CommandId) -> Option<CommandStatus> {
        self.cache.get(&command_id).map(|(_, _, status)| *status)
    }

    /// Number of commands currently tracked in cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns true if cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for TransientCommandDeduplicator {
    fn default() -> Self {
        Self::new(10_000)
    }
}

/// Priority queue storing ingress commands sorted deterministically by (assigned_tick, ingress_seq, command_id).
#[derive(Debug, Default, Clone)]
pub struct CommandQueue {
    queue: Vec<IngressCommand>,
}

impl CommandQueue {
    #[must_use]
    pub fn new() -> Self {
        Self { queue: Vec::new() }
    }

    pub fn enqueue(&mut self, cmd: IngressCommand) {
        self.queue.push(cmd);
    }

    pub fn drain_for_tick(&mut self, tick: u64) -> Vec<IngressCommand> {
        let (mut matching, remaining): (Vec<_>, Vec<_>) = self
            .queue
            .drain(..)
            .partition(|cmd| cmd.assigned_tick == tick);
        self.queue = remaining;

        // Strict deterministic sorting by ingress_seq then full 128-bit command_id
        matching.sort_by(|a, b| {
            a.ingress_seq
                .cmp(&b.ingress_seq)
                .then_with(|| a.command_id.cmp(&b.command_id))
        });
        matching
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
