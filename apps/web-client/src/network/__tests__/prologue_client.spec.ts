import { describe, it, expect, beforeEach } from 'vitest';
import { GameClient } from '../client.js';
import type { ServerMessage } from '../protocol.js';
import { loadClaimedReceipts, saveClaimedReceipts, STORAGE_KEY_RECEIPTS } from '../receipt_store.js';

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  public sent: string[] = [];
  public onopen: (() => void) | null = null;
  public onmessage: ((event: { data: string }) => void) | null = null;
  public onclose: (() => void) | null = null;
  public onerror: ((err: any) => void) | null = null;

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    if (this.onclose) this.onclose();
  }
}

describe('Prologue & World Transfer Client Integration (TASK-012A & 012B)', () => {
  let client: GameClient;
  let mockWs: MockWebSocket;

  beforeEach(() => {
    MockWebSocket.instances = [];
    (globalThis as any).WebSocket = MockWebSocket;

    client = new GameClient({
      wsUrl: 'ws://localhost:9000/ws',
      ticket: 'test_ticket_token',
      autoReconnect: false,
      heartbeatIntervalMs: 10000,
    });
  });

  function getActiveMockWs(): MockWebSocket {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }

  function simulateAuthSuccess(actorId = 'hero_01', regionId = 'reg_01') {
    client.connect();
    mockWs = getActiveMockWs();
    mockWs.onopen?.();

    const authOkMsg: ServerMessage = {
      type: 'auth_ok',
      payload: {
        session_id: 'sess_123',
        actor_id: actorId,
        region_id: regionId,
        protocol_version: 1,
      },
    };
    mockWs.onmessage?.({ data: JSON.stringify(authOkMsg) });
  }

  it('queries prologue state and sends start prologue command', () => {
    simulateAuthSuccess();

    client.queryPrologueState();
    const queryMsg = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(queryMsg.type).toBe('prologue_query_state');

    client.startPrologue();
    const startMsg = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(startMsg.type).toBe('prologue_start');
  });

  it('dispatches prologue_state_sync to listeners and verifies progress', () => {
    simulateAuthSuccess();

    let receivedSync: any = null;
    client.on('prologue_state_sync', (payload) => {
      receivedSync = payload;
    });

    const syncMsg: ServerMessage = {
      type: 'prologue_state_sync',
      payload: {
        state: 'IN_PROGRESS',
        step: 'P1_RESOURCE_GATHERING',
        progress: 2,
        target_count: 3,
        step_title: '자원 채집',
        step_description: '야생 약초를 채집하세요.',
        fallback_active: false,
        is_completed: false,
      },
    };

    mockWs.onmessage?.({ data: JSON.stringify(syncMsg) });

    expect(receivedSync).not.toBeNull();
    expect(receivedSync.state).toBe('IN_PROGRESS');
    expect(receivedSync.progress).toBe(2);
    expect(receivedSync.target_count).toBe(3);
    expect(receivedSync.is_completed).toBe(false);
  });

  it('sends step actions for P0 through P5 progression', () => {
    simulateAuthSuccess();

    client.sendPrologueStepAction('talk_to_npc', 'Elder Elda');
    let lastSent = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(lastSent.type).toBe('prologue_step_action');
    expect(lastSent.payload.action_type).toBe('talk_to_npc');
    expect(lastSent.payload.target_id).toBe('Elder Elda');

    client.sendPrologueStepAction('gather', 'wild_herbs');
    lastSent = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(lastSent.payload.action_type).toBe('gather');

    client.sendPrologueStepAction('defeat_encounter', 'goblin_scout');
    lastSent = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(lastSent.payload.action_type).toBe('defeat_encounter');
  });

  it('triggers movement failure fallback and parses fallback state (TEST-PRO-005)', () => {
    simulateAuthSuccess();

    client.triggerPrologueFallback('BlockedPath: Outpost bridge collapsed');
    const lastSent = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(lastSent.type).toBe('prologue_fallback');
    expect(lastSent.payload.reason).toContain('BlockedPath');

    let fallbackPayload: any = null;
    client.on('prologue_state_sync', (p) => {
      fallbackPayload = p;
    });

    const alertMsg: ServerMessage = {
      type: 'prologue_state_sync',
      payload: {
        state: 'IN_PROGRESS',
        step: 'P2_OUTPOST_TRAVEL',
        progress: 0,
        target_count: 1,
        step_title: '우회로 탐색',
        step_description: '다리가 무너졌습니다. 산길로 우회하세요.',
        fallback_active: true,
        last_failure_reason: 'BlockedPath: Outpost bridge collapsed',
        is_completed: false,
      },
    };
    mockWs.onmessage?.({ data: JSON.stringify(alertMsg) });

    expect(fallbackPayload.fallback_active).toBe(true);
    expect(fallbackPayload.last_failure_reason).toContain('BlockedPath');
  });

  it('handles world transfer request and parses transfer result (TEST-PRO-001, 003)', () => {
    simulateAuthSuccess();

    client.requestPrologueTransfer('target_shared_settlement_uuid');
    const lastSent = JSON.parse(mockWs.sent[mockWs.sent.length - 1]);
    expect(lastSent.type).toBe('prologue_request_transfer');
    expect(lastSent.payload.target_region_id).toBe('target_shared_settlement_uuid');

    let transferResult: any = null;
    client.on('prologue_transfer_result', (p) => {
      transferResult = p;
    });

    const resultMsg: ServerMessage = {
      type: 'prologue_transfer_result',
      payload: {
        transfer_id: 'xfr_9999',
        status: 'COMMITTED',
        target_region_id: 'target_shared_settlement_uuid',
        rewards: [
          {
            receipt_id: 'rcpt_1',
            sku: 'settlement_gold_100',
            quantity: 1,
            granted_at_ms: Date.now(),
          },
        ],
      },
    };
    mockWs.onmessage?.({ data: JSON.stringify(resultMsg) });

    expect(transferResult).not.toBeNull();
    expect(transferResult.status).toBe('COMMITTED');
    expect(transferResult.rewards.length).toBe(1);
  });

  it('verifies that completed prologue sets is_completed flag (TEST-PRO-004)', () => {
    simulateAuthSuccess();

    let isCompletedFlag = false;
    client.on('prologue_state_sync', (sync) => {
      isCompletedFlag = sync.is_completed;
    });

    const completedSync: ServerMessage = {
      type: 'prologue_state_sync',
      payload: {
        state: 'COMPLETED',
        step: 'COMPLETED',
        progress: 1,
        target_count: 1,
        step_title: '프롤로그 완료',
        step_description: '공유 세계에 안착했습니다.',
        fallback_active: false,
        is_completed: true,
      },
    };
    mockWs.onmessage?.({ data: JSON.stringify(completedSync) });

    expect(isCompletedFlag).toBe(true);
  });

  it('persists claimed receipts to localStorage and ignores duplicates across simulated reloads', () => {
    const memoryStorage: Record<string, string> = {};
    const mockStorage: Storage = {
      getItem: (key: string) => memoryStorage[key] ?? null,
      setItem: (key: string, value: string) => {
        memoryStorage[key] = value;
      },
      removeItem: (key: string) => {
        delete memoryStorage[key];
      },
      clear: () => {
        Object.keys(memoryStorage).forEach((k) => delete memoryStorage[k]);
      },
      key: (index: number) => Object.keys(memoryStorage)[index] ?? null,
      length: 0,
    };

    // First session: initial load is empty
    const session1Receipts = loadClaimedReceipts(mockStorage);
    expect(session1Receipts.size).toBe(0);

    // Apply receipt
    session1Receipts.add('rcpt_100_gold');
    saveClaimedReceipts(session1Receipts, mockStorage);

    // Verify written to storage
    expect(mockStorage.getItem(STORAGE_KEY_RECEIPTS)).toContain('rcpt_100_gold');

    // Second session (simulated page reload):
    const session2Receipts = loadClaimedReceipts(mockStorage);
    expect(session2Receipts.has('rcpt_100_gold')).toBe(true);
    expect(session2Receipts.size).toBe(1);

    // Duplicate arrival in session 2:
    const incomingReceipts = ['rcpt_100_gold', 'rcpt_200_supplies'];
    let newlyClaimed = 0;
    for (const r of incomingReceipts) {
      if (!session2Receipts.has(r)) {
        session2Receipts.add(r);
        newlyClaimed++;
      }
    }
    saveClaimedReceipts(session2Receipts, mockStorage);

    expect(newlyClaimed).toBe(1); // Only rcpt_200_supplies was newly claimed
    expect(session2Receipts.size).toBe(2);

    // Third session:
    const session3Receipts = loadClaimedReceipts(mockStorage);
    expect(session3Receipts.has('rcpt_100_gold')).toBe(true);
    expect(session3Receipts.has('rcpt_200_supplies')).toBe(true);
  });
});
