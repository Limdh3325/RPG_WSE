//! Wire protocol v1 definitions for RPG WSE WebSocket communication.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireCommandPayload {
    Move { target_x: i64, target_y: i64 },
    SetVelocity { dx: i32, dy: i32 },
    Attack { target_id: Uuid },
    Purchase { item_id: u64, price: i64 },
    Custom { action: String, payload: Vec<u8> },
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsTicketRequest {
    pub account_id: Uuid,
    pub actor_id: Uuid,
    pub region_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsTicketResponse {
    pub ticket: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityStateWire {
    pub id: Uuid,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub entity_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireCommandStatus {
    Accepted,
    Rejected,
    Committed,
    Failed,
}

/// Messages sent from Client to Gateway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    Auth {
        ticket: String,
    },
    SnapshotAck {
        snapshot_id: Uuid,
        world_revision: u64,
    },
    DeltaAck {
        delta_seq: u64,
    },
    Command {
        command_id: Uuid,
        actor_id: Uuid,
        payload: WireCommandPayload,
    },
    ResyncRequest {
        reason: String,
        last_seen_revision: u64,
    },
    Ping {
        nonce: u64,
    },
    PrologueQueryState,
    PrologueStart,
    PrologueStepAction {
        action_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<String>,
    },
    PrologueFallback {
        reason: String,
    },
    PrologueRequestTransfer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_region_id: Option<Uuid>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrologueRewardWire {
    pub receipt_id: Uuid,
    pub sku: String,
    pub quantity: u32,
    pub granted_at_ms: i64,
}

/// Messages sent from Gateway to Client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerMessage {
    AuthOk {
        session_id: Uuid,
        actor_id: Uuid,
        region_id: Uuid,
        protocol_version: u32,
    },
    AuthError {
        code: String,
        message: String,
    },
    WorldSnapshot {
        snapshot_id: Uuid,
        world_revision: u64,
        region_tick: u64,
        entities: Vec<EntityStateWire>,
    },
    StateDelta {
        baseline_id: Uuid,
        delta_seq: u64,
        region_tick: u64,
        changed: Vec<EntityStateWire>,
        removed: Vec<Uuid>,
    },
    EntityEnter {
        entity: EntityStateWire,
    },
    EntityLeave {
        entity_id: Uuid,
    },
    CommandStatus {
        command_id: Uuid,
        status: WireCommandStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        committed_event_seq: Option<u64>,
    },
    ResyncRequired {
        reason: String,
        current_revision: u64,
    },
    Pong {
        nonce: u64,
    },
    PrologueStateSync {
        state: String,
        step: String,
        progress: u32,
        target_count: u32,
        step_title: String,
        step_description: String,
        fallback_active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_failure_reason: Option<String>,
        is_completed: bool,
    },
    PrologueTransferResult {
        transfer_id: Uuid,
        status: String,
        target_region_id: Uuid,
        rewards: Vec<PrologueRewardWire>,
    },
}
