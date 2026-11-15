import { describe, it, expect, vi, beforeEach } from 'vitest';
import { GameClient } from '../client.js';
import type { ServerMessage } from '../protocol.js';

// Mock WebSocket
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  public readyState = 1; // OPEN
  public onopen: (() => void) | null = null;
  public onmessage: ((event: { data: string }) => void) | null = null;
  public onclose: ((event: { code: number; reason: string }) => void) | null = null;
  public onerror: ((error: any) => void) | null = null;
  public sent: string[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => this.onopen?.(), 0);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close(code = 1000, reason = 'normal') {
    this.readyState = 3; // CLOSED
    this.onclose?.({ code, reason });
  }

  receiveMessage(msg: ServerMessage) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
}

(globalThis as any).WebSocket = MockWebSocket;

describe('GameClient Network & Protocol Unit Tests (Rust shared-protocol v1 compliant)', () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.clearAllMocks();
  });

  it('connects and immediately sends Auth ticket with serde tag/content structure', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'valid_ticket_abc',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));

    const ws = MockWebSocket.instances[0];
    expect(ws).toBeDefined();
    expect(ws.sent.length).toBeGreaterThanOrEqual(1);

    const firstMsg = JSON.parse(ws.sent[0]);
    expect(firstMsg).toEqual({
      type: 'auth',
      payload: {
        ticket: 'valid_ticket_abc',
      },
    });

    client.disconnect();
  });

  it('processes auth_ok and starts ping heartbeat', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'ticket_123',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const ws = MockWebSocket.instances[0];

    const authOkMsg: ServerMessage = {
      type: 'auth_ok',
      payload: {
        session_id: '11111111-1111-1111-1111-111111111111',
        actor_id: '22222222-2222-2222-2222-222222222222',
        region_id: '33333333-3333-3333-3333-333333333333',
        protocol_version: 1,
      },
    };

    ws.receiveMessage(authOkMsg);

    expect(client.getState()).toBe('connected');
    expect(client.getSessionId()).toBe('11111111-1111-1111-1111-111111111111');
    expect(client.getActorId()).toBe('22222222-2222-2222-2222-222222222222');
    expect(client.getRegionId()).toBe('33333333-3333-3333-3333-333333333333');

    // Heartbeat ping should be sent
    const pingMsg = ws.sent.map((s) => JSON.parse(s)).find((m) => m.type === 'ping');
    expect(pingMsg).toBeDefined();
    expect(typeof pingMsg.payload.nonce).toBe('number');

    client.disconnect();
  });

  it('handles world_snapshot and sends snapshot_ack', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'valid_ticket',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const ws = MockWebSocket.instances[0];

    const snapshotMsg: ServerMessage = {
      type: 'world_snapshot',
      payload: {
        snapshot_id: 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
        world_revision: 42,
        region_tick: 500,
        entities: [
          {
            id: 'actor-1',
            x: 10,
            y: 20,
            entity_type: 'player',
            hp: 100,
            max_hp: 100,
            name: 'Hero',
          },
          {
            id: 'monster-1',
            x: 50,
            y: 60,
            entity_type: 'goblin',
            hp: 30,
            max_hp: 30,
            name: 'Goblin Scout',
          },
        ],
      },
    };

    ws.receiveMessage(snapshotMsg);

    const entities = client.getEntities();
    expect(entities.length).toBe(2);
    expect(entities.find((e) => e.id === 'actor-1')?.x).toBe(10);
    expect(entities.find((e) => e.id === 'monster-1')?.name).toBe('Goblin Scout');
    expect(client.getWorldRevision()).toBe(42);

    // Verify snapshot_ack
    const ackMsg = ws.sent.map((s) => JSON.parse(s)).find((m) => m.type === 'snapshot_ack');
    expect(ackMsg).toBeDefined();
    expect(ackMsg.payload.snapshot_id).toBe('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa');
    expect(ackMsg.payload.world_revision).toBe(42);

    client.disconnect();
  });

  it('handles state_delta, updates entities, and sends delta_ack', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'valid_ticket',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const ws = MockWebSocket.instances[0];

    // Initial snapshot
    ws.receiveMessage({
      type: 'world_snapshot',
      payload: {
        snapshot_id: 'snap-1',
        world_revision: 10,
        region_tick: 100,
        entities: [
          { id: 'actor-1', x: 0, y: 0, entity_type: 'player', hp: 100, max_hp: 100 },
          { id: 'actor-2', x: 10, y: 10, entity_type: 'player', hp: 80, max_hp: 80 },
        ],
      },
    });

    // Delta seq 1
    ws.receiveMessage({
      type: 'state_delta',
      payload: {
        baseline_id: 'snap-1',
        delta_seq: 1,
        region_tick: 101,
        changed: [
          { id: 'actor-1', x: 5, y: 5, entity_type: 'player', hp: 90, max_hp: 100 },
        ],
        removed: ['actor-2'],
      },
    });

    const entities = client.getEntities();
    expect(entities.length).toBe(1);
    expect(entities[0].id).toBe('actor-1');
    expect(entities[0].x).toBe(5);
    expect(entities[0].hp).toBe(90);

    const deltaAck = ws.sent.map((s) => JSON.parse(s)).find((m) => m.type === 'delta_ack');
    expect(deltaAck).toBeDefined();
    expect(deltaAck.payload.delta_seq).toBe(1);

    client.disconnect();
  });

  it('detects delta sequence gap and triggers resync_request', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'valid_ticket',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const ws = MockWebSocket.instances[0];

    // Seed with delta_seq 1
    ws.receiveMessage({
      type: 'world_snapshot',
      payload: {
        snapshot_id: 'snap-1',
        world_revision: 10,
        region_tick: 100,
        entities: [],
      },
    });

    ws.receiveMessage({
      type: 'state_delta',
      payload: {
        baseline_id: 'snap-1',
        delta_seq: 1,
        region_tick: 101,
        changed: [],
        removed: [],
      },
    });

    // Receive delta_seq 5 instead of 2 (gap!)
    ws.receiveMessage({
      type: 'state_delta',
      payload: {
        baseline_id: 'snap-1',
        delta_seq: 5,
        region_tick: 105,
        changed: [],
        removed: [],
      },
    });

    const resync = ws.sent.map((s) => JSON.parse(s)).find((m) => m.type === 'resync_request');
    expect(resync).toBeDefined();
    expect(resync.payload.last_seen_revision).toBe(10);
    expect(resync.payload.reason).toContain('Delta sequence gap');

    client.disconnect();
  });

  it('sends Move and Attack commands in serde tag/payload format', async () => {
    const client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'valid_ticket',
    });

    client.connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const ws = MockWebSocket.instances[0];

    // Fake auth
    ws.receiveMessage({
      type: 'auth_ok',
      payload: {
        session_id: 'session-123',
        actor_id: 'player-hero',
        region_id: 'dungeon-1',
        protocol_version: 1,
      },
    });

    // Send Move
    const moveCmdId = client.sendMove(32, 64);
    const moveMsg = ws.sent.map((s) => JSON.parse(s)).find(
      (m) => m.type === 'command' && m.payload.payload.kind === 'move'
    );
    expect(moveMsg).toBeDefined();
    expect(moveMsg.payload.command_id).toBe(moveCmdId);
    expect(moveMsg.payload.actor_id).toBe('player-hero');
    expect(moveMsg.payload.payload).toEqual({
      kind: 'move',
      target_x: 32,
      target_y: 64,
    });

    // Send Attack
    const attackCmdId = client.sendAttack('boss-orc');
    const attackMsg = ws.sent.map((s) => JSON.parse(s)).find(
      (m) => m.type === 'command' && m.payload.payload.kind === 'attack'
    );
    expect(attackMsg).toBeDefined();
    expect(attackMsg.payload.command_id).toBe(attackCmdId);
    expect(attackMsg.payload.payload).toEqual({
      kind: 'attack',
      target_id: 'boss-orc',
    });

    client.disconnect();
  });

  it('receives command_status server message and triggers listeners', async () => {
    const client = new GameClient({ wsUrl: 'ws://localhost:8080/ws', ticket: 'ticket-status-test' });
    client.connect();

    const ws = (client as any).ws as MockWebSocket;

    let receivedStatus: any = null;
    client.on('command_status', (status) => {
      receivedStatus = status;
    });

    // Simulate backend sending ServerMessage::CommandStatus
    ws.receiveMessage({
      type: 'command_status',
      payload: {
        command_id: 'cmd-1234-5678',
        status: 'accepted',
        committed_event_seq: 42,
      },
    });

    expect(receivedStatus).toEqual({
      command_id: 'cmd-1234-5678',
      status: 'accepted',
      committed_event_seq: 42,
    });

    client.disconnect();
  });
});
