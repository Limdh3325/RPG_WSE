use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiptStatus {
    Pending,
    Succeeded,
    Failed,
    Rejected,
}

impl ReceiptStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Rejected => "REJECTED",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s {
            "PENDING" => Some(Self::Pending),
            "SUCCEEDED" => Some(Self::Succeeded),
            "FAILED" => Some(Self::Failed),
            "REJECTED" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandReceiptRecord {
    pub command_id: Uuid,
    pub world_id: Uuid,
    pub region_id: Uuid,
    pub actor_id: Uuid,
    pub payload_hash: Vec<u8>,
    pub status: ReceiptStatus,
    pub committed_tick: Option<i64>,
    pub result_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DurableOperation {
    WalletTransfer {
        wallet_id: Uuid,
        delta: i64,
    },
    MarketPurchase {
        settlement_id: Uuid,
        buyer_wallet_id: Uuid,
        seller_wallet_id: Option<Uuid>,
        sku: String,
        quantity: i64,
        unit_price: i64,
    },
    InventoryTransfer {
        entity_id: Uuid,
        slot_index: i16,
        item_id: Option<Uuid>,
        sku: String,
        quantity: i32,
        durability: i16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableEvent {
    pub id: Uuid,
    pub stream_type: String,
    pub aggregate_id: Uuid,
    pub event_type: String,
    pub topic: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableIntent {
    pub command_id: Uuid,
    pub world_id: Uuid,
    pub region_id: Uuid,
    pub ownership_epoch: i64,
    pub actor_id: Uuid,
    pub payload_hash: [u8; 32],
    pub tick: u64,
    pub operations: Vec<DurableOperation>,
    pub emitted_events: Vec<DurableEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceAck {
    pub command_id: Uuid,
    pub region_id: Uuid,
    pub status: ReceiptStatus,
    pub committed_seq: Option<i64>,
    pub result_json: serde_json::Value,
}
