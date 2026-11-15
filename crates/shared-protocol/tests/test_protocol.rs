use shared_protocol::{ClientMessage, PrologueRewardWire, ServerMessage};
use uuid::Uuid;

#[test]
fn test_client_message_prologue_serialization() {
    let msg = ClientMessage::PrologueStart;
    let serialized = serde_json::to_string(&msg).expect("serialize PrologueStart");
    assert!(serialized.contains("\"type\":\"prologue_start\""));

    let action_msg = ClientMessage::PrologueStepAction {
        action_type: "talk_to_npc".to_string(),
        target_id: Some("elder_elda".to_string()),
        payload: None,
    };
    let serialized_action = serde_json::to_string(&action_msg).expect("serialize action");
    let deserialized_action: ClientMessage =
        serde_json::from_str(&serialized_action).expect("deserialize action");
    assert_eq!(action_msg, deserialized_action);
}

#[test]
fn test_server_message_prologue_serialization() {
    let transfer_id = Uuid::new_v4();
    let target_region_id = Uuid::new_v4();
    let receipt_id = Uuid::new_v4();

    let state_msg = ServerMessage::PrologueStateSync {
        state: "IN_PROGRESS".to_string(),
        step: "P0_VILLAGE_ENTRY".to_string(),
        progress: 0,
        target_count: 1,
        step_title: "Village Arrival".to_string(),
        step_description: "Meet Elder Elda".to_string(),
        fallback_active: false,
        last_failure_reason: None,
        is_completed: false,
    };

    let serialized_state = serde_json::to_string(&state_msg).expect("serialize state sync");
    let deserialized_state: ServerMessage =
        serde_json::from_str(&serialized_state).expect("deserialize state sync");
    assert_eq!(state_msg, deserialized_state);

    let result_msg = ServerMessage::PrologueTransferResult {
        transfer_id,
        status: "COMMITTED".to_string(),
        target_region_id,
        rewards: vec![PrologueRewardWire {
            receipt_id,
            sku: "potion_starter".to_string(),
            quantity: 3,
            granted_at_ms: 1234567,
        }],
    };

    let serialized_result = serde_json::to_string(&result_msg).expect("serialize transfer result");
    let deserialized_result: ServerMessage =
        serde_json::from_str(&serialized_result).expect("deserialize transfer result");
    assert_eq!(result_msg, deserialized_result);
}
