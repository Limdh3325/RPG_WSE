/**
 * Shared Wire Protocol v1 Definitions
 * 1:1 mapping with Rust `crates/shared-protocol/src/lib.rs`
 */

export const PROTOCOL_VERSION = 1;

export type WireCommandStatus = 'accepted' | 'rejected' | 'committed' | 'failed';

export type WireCommandPayload =
  | { kind: 'move'; target_x: number; target_y: number }
  | { kind: 'set_velocity'; dx: number; dy: number }
  | { kind: 'attack'; target_id: string }
  | { kind: 'purchase'; item_id: number; price: number }
  | { kind: 'custom'; action: string; payload: number[] }
  | { kind: 'noop' };

export interface EntityStateWire {
  id: string;
  x: number;
  y: number;
  entity_type: string;
  hp?: number;
  max_hp?: number;
  name?: string;
}

export interface PrologueRewardWire {
  receipt_id: string;
  sku: string;
  quantity: number;
  granted_at_ms: number;
}

// Client Messages (tagged with type, content in payload)
export type ClientMessage =
  | { type: 'auth'; payload: { ticket: string } }
  | { type: 'snapshot_ack'; payload: { snapshot_id: string; world_revision: number } }
  | { type: 'delta_ack'; payload: { delta_seq: number } }
  | {
      type: 'command';
      payload: {
        command_id: string;
        actor_id: string;
        payload: WireCommandPayload;
      };
    }
  | { type: 'resync_request'; payload: { reason: string; last_seen_revision: number } }
  | { type: 'ping'; payload: { nonce: number } }
  | { type: 'prologue_query_state' }
  | { type: 'prologue_start' }
  | {
      type: 'prologue_step_action';
      payload: {
        action_type: string;
        target_id?: string;
        payload?: string;
      };
    }
  | { type: 'prologue_fallback'; payload: { reason: string } }
  | { type: 'prologue_request_transfer'; payload: { target_region_id?: string } };

// Server Messages (tagged with type, content in payload)
export type ServerMessage =
  | {
      type: 'auth_ok';
      payload: {
        session_id: string;
        actor_id: string;
        region_id: string;
        protocol_version: number;
      };
    }
  | {
      type: 'auth_error';
      payload: {
        code: string;
        message: string;
      };
    }
  | {
      type: 'world_snapshot';
      payload: {
        snapshot_id: string;
        world_revision: number;
        region_tick: number;
        entities: EntityStateWire[];
      };
    }
  | {
      type: 'state_delta';
      payload: {
        baseline_id: string;
        delta_seq: number;
        region_tick: number;
        changed: EntityStateWire[];
        removed: string[];
      };
    }
  | {
      type: 'entity_enter';
      payload: {
        entity: EntityStateWire;
      };
    }
  | {
      type: 'entity_leave';
      payload: {
        entity_id: string;
      };
    }
  | {
      type: 'command_status';
      payload: {
        command_id: string;
        status: WireCommandStatus;
        reason?: string;
        committed_event_seq?: number;
      };
    }
  | {
      type: 'resync_required';
      payload: {
        reason: string;
        current_revision: number;
      };
    }
  | {
      type: 'pong';
      payload: {
        nonce: number;
      };
    }
  | {
      type: 'prologue_state_sync';
      payload: {
        state: string;
        step: string;
        progress: number;
        target_count: number;
        step_title: string;
        step_description: string;
        fallback_active: boolean;
        last_failure_reason?: string;
        is_completed: boolean;
      };
    }
  | {
      type: 'prologue_transfer_result';
      payload: {
        transfer_id: string;
        status: string;
        target_region_id: string;
        rewards: PrologueRewardWire[];
      };
    }
  | {
      type: 'announcement';
      payload: {
        title: string;
        message: string;
      };
    };
