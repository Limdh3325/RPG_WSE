import {
  type ClientMessage,
  type ServerMessage,
  type EntityStateWire,
  type WireCommandPayload,
} from './protocol.js';

export type ClientState = 'disconnected' | 'connecting' | 'authenticating' | 'connected';

export interface GameClientOptions {
  wsUrl: string;
  ticket: string;
  autoReconnect?: boolean;
  heartbeatIntervalMs?: number;
}

export type EventCallback = (...args: any[]) => void;

function generateUuid(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

export class GameClient {
  private ws: WebSocket | null = null;
  private state: ClientState = 'disconnected';
  private options: GameClientOptions;
  private listeners: Map<string, Set<EventCallback>> = new Map();
  private pingIntervalId: any = null;

  // Authoritative state synchronized from server
  private sessionId: string | null = null;
  private actorId: string | null = null;
  private regionId: string | null = null;
  private entities: Map<string, EntityStateWire> = new Map();
  private worldRevision: number = 0;
  private lastSnapshotId: string | null = null;
  private lastDeltaSeq: number = 0;
  private lastTick: number = 0;
  private lastRtt: number = 0;

  constructor(options: GameClientOptions) {
    this.options = {
      autoReconnect: false,
      heartbeatIntervalMs: 5000,
      ...options,
    };
  }

  public getState(): ClientState {
    return this.state;
  }

  public getSessionId(): string | null {
    return this.sessionId;
  }

  public getActorId(): string | null {
    return this.actorId;
  }

  public getRegionId(): string | null {
    return this.regionId;
  }

  public getEntities(): EntityStateWire[] {
    return Array.from(this.entities.values());
  }

  public getEntity(id: string): EntityStateWire | undefined {
    return this.entities.get(id);
  }

  public getWorldRevision(): number {
    return this.worldRevision;
  }

  public getLastDeltaSeq(): number {
    return this.lastDeltaSeq;
  }

  public getLastSnapshotSeq(): number {
    return this.worldRevision;
  }

  public getLastTick(): number {
    return this.lastTick;
  }

  public getLastRtt(): number {
    return this.lastRtt;
  }

  public getLastSnapshotId(): string | null {
    return this.lastSnapshotId;
  }

  public connect(): void {
    if (this.state === 'connected' || this.state === 'connecting') {
      return;
    }

    this.state = 'connecting';
    this.emit('state_change', this.state);

    try {
      this.ws = new WebSocket(this.options.wsUrl);
      this.setupSocketHandlers();
    } catch (err) {
      this.state = 'disconnected';
      this.emit('state_change', this.state);
      this.emit('error', err);
    }
  }

  public disconnect(): void {
    this.stopHeartbeat();
    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;
      this.ws.close();
      this.ws = null;
    }
    this.state = 'disconnected';
    this.emit('state_change', this.state);
  }

  public on(event: string, callback: EventCallback): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(callback);
  }

  public off(event: string, callback: EventCallback): void {
    this.listeners.get(event)?.delete(callback);
  }

  public emit(event: string, ...args: any[]): void {
    const handlers = this.listeners.get(event);
    if (handlers) {
      handlers.forEach((fn) => {
        try {
          fn(...args);
        } catch (e) {
          console.error(`Error in event listener for ${event}:`, e);
        }
      });
    }
  }

  public send(msg: ClientMessage): boolean {
    const isWsOpen = typeof WebSocket !== 'undefined' ? WebSocket.OPEN : 1;
    if (!this.ws || this.ws.readyState !== isWsOpen) {
      return false;
    }
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  public sendCommand(payload: WireCommandPayload): string | null {
    if (!this.actorId) {
      console.warn('Cannot send command before auth_ok');
      return null;
    }
    const commandId = generateUuid();
    const msg: ClientMessage = {
      type: 'command',
      payload: {
        command_id: commandId,
        actor_id: this.actorId,
        payload,
      },
    };
    if (this.send(msg)) {
      return commandId;
    }
    return null;
  }

  public sendMove(targetX: number, targetY: number): string | null {
    return this.sendCommand({
      kind: 'move',
      target_x: Math.round(targetX),
      target_y: Math.round(targetY),
    });
  }

  public sendAttack(targetId: string): string | null {
    return this.sendCommand({
      kind: 'attack',
      target_id: targetId,
    });
  }

  public sendResyncRequest(reason: string = 'client_manual_resync'): void {
    this.send({
      type: 'resync_request',
      payload: {
        reason,
        last_seen_revision: this.worldRevision,
      },
    });
  }

  public queryPrologueState(): void {
    this.send({ type: 'prologue_query_state' });
  }

  public startPrologue(): void {
    this.send({ type: 'prologue_start' });
  }

  public sendPrologueStepAction(actionType: string, targetId?: string, payload?: string): void {
    this.send({
      type: 'prologue_step_action',
      payload: {
        action_type: actionType,
        target_id: targetId,
        payload,
      },
    });
  }

  public triggerPrologueFallback(reason: string): void {
    this.send({
      type: 'prologue_fallback',
      payload: { reason },
    });
  }

  public requestPrologueTransfer(targetRegionId?: string): void {
    this.send({
      type: 'prologue_request_transfer',
      payload: { target_region_id: targetRegionId },
    });
  }

  private setupSocketHandlers(): void {
    if (!this.ws) return;

    this.ws.onopen = () => {
      this.state = 'authenticating';
      this.emit('state_change', this.state);

      // 1. Send auth ticket to gateway
      const authMsg: ClientMessage = {
        type: 'auth',
        payload: {
          ticket: this.options.ticket,
        },
      };
      this.send(authMsg);
    };

    this.ws.onmessage = (event: MessageEvent) => {
      try {
        const msg = JSON.parse(event.data) as ServerMessage;
        this.handleServerMessage(msg);
      } catch (err) {
        console.error('Failed to parse incoming WebSocket message:', err);
      }
    };

    this.ws.onerror = (err) => {
      this.emit('error', err);
    };

    this.ws.onclose = () => {
      this.stopHeartbeat();
      this.state = 'disconnected';
      this.emit('state_change', this.state);
    };
  }

  private handleServerMessage(msg: ServerMessage): void {
    switch (msg.type) {
      case 'auth_ok': {
        this.sessionId = msg.payload.session_id;
        this.actorId = msg.payload.actor_id;
        this.regionId = msg.payload.region_id;
        this.state = 'connected';
        this.emit('state_change', this.state);
        this.emit('authenticated', msg.payload);
        this.startHeartbeat();
        break;
      }

      case 'auth_error': {
        this.emit('auth_error', msg.payload);
        this.disconnect();
        break;
      }

      case 'world_snapshot': {
        const { snapshot_id, world_revision, region_tick, entities } = msg.payload;
        this.entities.clear();
        for (const e of entities) {
          this.entities.set(e.id, e);
        }
        this.worldRevision = world_revision;
        this.lastSnapshotId = snapshot_id;
        this.lastTick = region_tick;

        // Auto-reply SnapshotAck
        this.send({
          type: 'snapshot_ack',
          payload: {
            snapshot_id,
            world_revision,
          },
        });

        this.emit('snapshot', msg.payload);
        break;
      }

      case 'state_delta': {
        const { delta_seq, region_tick, changed, removed } = msg.payload;

        // Detect sequence gap
        if (this.lastDeltaSeq > 0 && delta_seq !== this.lastDeltaSeq + 1) {
          console.warn(`Delta sequence gap detected: expected ${this.lastDeltaSeq + 1}, got ${delta_seq}`);
          this.sendResyncRequest('Delta sequence gap detected');
        }

        this.lastDeltaSeq = delta_seq;
        this.lastTick = region_tick;

        for (const ent of changed) {
          this.entities.set(ent.id, ent);
        }
        for (const remId of removed) {
          this.entities.delete(remId);
        }

        // Auto-reply DeltaAck
        this.send({
          type: 'delta_ack',
          payload: {
            delta_seq,
          },
        });

        this.emit('delta', msg.payload);
        break;
      }

      case 'entity_enter': {
        this.entities.set(msg.payload.entity.id, msg.payload.entity);
        this.emit('entity_enter', msg.payload.entity);
        break;
      }

      case 'entity_leave': {
        this.entities.delete(msg.payload.entity_id);
        this.emit('entity_leave', msg.payload.entity_id);
        break;
      }

      case 'command_status': {
        this.emit('command_status', msg.payload);
        this.emit('command_ack', msg.payload); // Backward compatibility
        break;
      }

      case 'resync_required': {
        this.emit('resync_required', msg.payload);
        this.sendResyncRequest(msg.payload.reason);
        break;
      }

      case 'pong': {
        const rtt = Date.now() - msg.payload.nonce;
        this.lastRtt = Math.max(0, rtt);
        this.emit('pong', { nonce: msg.payload.nonce, rtt: this.lastRtt });
        break;
      }

      case 'prologue_state_sync': {
        this.emit('prologue_state_sync', msg.payload);
        break;
      }

      case 'prologue_transfer_result': {
        this.emit('prologue_transfer_result', msg.payload);
        break;
      }

      default:
        console.warn('Unhandled server message:', msg);
    }
  }

  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.send({
      type: 'ping',
      payload: { nonce: Date.now() },
    });
    this.pingIntervalId = setInterval(() => {
      this.send({
        type: 'ping',
        payload: { nonce: Date.now() },
      });
    }, this.options.heartbeatIntervalMs);
  }

  private stopHeartbeat(): void {
    if (this.pingIntervalId) {
      clearInterval(this.pingIntervalId);
      this.pingIntervalId = null;
    }
  }
}
