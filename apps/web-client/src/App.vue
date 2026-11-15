<template>
  <div class="game-wrapper">
    <!-- Top Header HUD -->
    <header class="top-hud">
      <div class="hud-section">
        <h1 class="logo">Project-New <span class="badge">WSE Client</span></h1>
        <div class="conn-status" :class="connectionStatus.toLowerCase()">
          <span class="status-dot"></span>
          <span>{{ connectionStatusText }}</span>
        </div>
        <div v-if="isConnected" class="stats">
          <span class="stat-item"><strong>Region:</strong> {{ regionId || '-' }}</span>
          <span class="stat-item"><strong>Tick:</strong> {{ currentTick }}</span>
          <span class="stat-item"><strong>Revision:</strong> {{ currentRev }}</span>
          <span class="stat-item"><strong>RTT:</strong> {{ rtt }}ms</span>
        </div>
      </div>

      <!-- Confirmed Wealth & Assets -->
      <div class="hud-section wealth-assets">
        <div class="asset-badge gold">
          <span class="asset-icon">🪙</span>
          <span class="asset-val">{{ gold }} Gold</span>
        </div>
        <div class="asset-badge supplies">
          <span class="asset-icon">📦</span>
          <span class="asset-val">{{ supplyPoints }} Supplies</span>
        </div>
      </div>

      <!-- Auth and Connection Controls -->
      <div class="hud-section auth-controls">
        <input :value="wsUrl" readonly class="input" title="Locked to gateway origin" placeholder="ws://.../ws" />
        <input v-model="actorId" class="input" placeholder="Actor ID (UUID)" />
        <button
          v-if="!isConnected"
          @click="handleConnect"
          :disabled="connectionStatus === 'CONNECTING'"
          class="btn btn-primary"
        >
          {{ connectionStatus === 'CONNECTING' ? 'Connecting...' : 'Connect' }}
        </button>
        <button v-else @click="handleDisconnect" class="btn btn-danger">Disconnect</button>
      </div>
    </header>

    <!-- Main Content Area -->
    <main class="main-content">
      <!-- Phaser Canvas Container & Overlays -->
      <div class="canvas-wrapper">
        <div id="phaser-container" class="canvas-container"></div>

        <!-- Command Status Floating Toast / Badge -->
        <div
          v-if="lastCommandFeedback"
          class="command-status-badge"
          :class="lastCommandFeedback.status.toLowerCase()"
        >
          <span class="status-dot"></span>
          <span>{{ lastCommandFeedback.text }}</span>
        </div>

        <!-- In-Game Dialogue Box Overlay -->
        <div v-if="showDialogue" class="dialogue-overlay">
          <div class="dialogue-box">
            <div class="dialogue-header">
              <span class="dialogue-avatar">🗣️</span>
              <strong class="dialogue-speaker">{{ dialogueNpcName }}</strong>
            </div>
            <p class="dialogue-body">{{ dialogueText }}</p>
            <div class="dialogue-footer">
              <button
                v-for="(choice, idx) in dialogueChoices"
                :key="idx"
                @click="choice.action"
                class="btn btn-sm btn-primary"
              >
                {{ choice.label }}
              </button>
              <button @click="closeDialogue" class="btn btn-sm btn-secondary">닫기 (Close)</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Right Overlay Sidebar Panel -->
      <aside class="sidebar-panel">
        <!-- Local Player Card -->
        <section class="card player-card">
          <h3>Local Actor</h3>
          <div v-if="localPlayer" class="player-stats">
            <div class="stat-row">
              <span class="lbl">ID:</span>
              <span class="val font-mono">{{ localPlayer.id }}</span>
            </div>
            <div class="stat-row">
              <span class="lbl">Pos:</span>
              <span class="val font-mono">({{ Math.round(localPlayer.x) }}, {{ Math.round(localPlayer.y) }})</span>
            </div>
            <div class="stat-row">
              <span class="lbl">HP:</span>
              <div class="hp-gauge">
                <div class="hp-fill" :style="{ width: `${hpPercent}%` }"></div>
                <span class="hp-text">{{ localPlayer.hp ?? 100 }} / {{ localPlayer.max_hp ?? 100 }}</span>
              </div>
            </div>
          </div>
          <div v-else class="empty-hint">Waiting for world snapshot...</div>
        </section>

        <!-- Prologue Tracker & Guide (TASK-012B) -->
        <section class="card prologue-card">
          <div class="prologue-header">
            <h3>프롤로그 가이드 (Prologue)</h3>
            <span class="prologue-status-tag" :class="prologueState.toLowerCase()">
              {{ prologueState }}
            </span>
          </div>

          <!-- If Already Completed: Hide Start Button -->
          <div v-if="isPrologueCompleted" class="prologue-completed-banner">
            <span class="check-icon">✓</span>
            <div>
              <strong>프롤로그 완료됨 (Prologue Completed)</strong>
              <p>공유 정착지 세계로 이관 완료되었습니다. (버튼 숨김)</p>
            </div>
          </div>

          <!-- In-Progress or Not Started -->
          <div v-else class="prologue-body">
            <div v-if="prologueState === 'NOT_STARTED'" class="prologue-not-started">
              <p class="guide-text">개인 개척촌(Private Region)에서 튜토리얼을 시작하세요.</p>
              <button
                @click="handleStartPrologue"
                class="btn btn-primary btn-block"
                :disabled="!isConnected"
              >
                프롤로그 여정 시작 (Start Prologue)
              </button>
            </div>

            <div v-else class="prologue-in-progress">
              <div class="step-title-row">
                <strong class="step-title">{{ prologueStepTitle }}</strong>
                <span class="step-badge">{{ prologueStep }}</span>
              </div>
              <p class="step-desc">{{ prologueStepDesc }}</p>

              <!-- Progress Bar -->
              <div v-if="prologueTargetCount > 0" class="progress-container">
                <div class="progress-bar-bg">
                  <div
                    class="progress-bar-fill"
                    :style="{ width: `${Math.min(100, Math.round((prologueProgress / prologueTargetCount) * 100))}%` }"
                  ></div>
                </div>
                <span class="progress-label">{{ prologueProgress }} / {{ prologueTargetCount }}</span>
              </div>

              <!-- Fallback Warning / Action (TEST-PRO-005, 006) -->
              <div v-if="prologueFallbackActive" class="prologue-fallback-alert">
                <span class="alert-icon">⚠️</span>
                <div class="alert-content">
                  <strong>대체 진행 경로 활성화</strong>
                  <p>{{ prologueFailureReason || '장애 발생으로 인해 우회 진행이 가능합니다.' }}</p>
                  <button @click="handleUseFallback" class="btn btn-warn btn-xs">
                    대체 수단으로 계속 진행
                  </button>
                </div>
              </div>

              <!-- Contextual Action Buttons -->
              <div class="prologue-actions">
                <button
                  v-if="prologueStep === 'P0_VILLAGE_ENTRY' || prologueStep === 'P0VillageEntry'"
                  @click="handleP0Action"
                  class="btn btn-sm btn-primary"
                  :disabled="!isConnected"
                >
                  촌장 엘다와 대화 (Talk)
                </button>
                <button
                  v-else-if="prologueStep === 'P1_RESOURCE_GATHERING' || prologueStep === 'P1ResourceGathering'"
                  @click="handleP1Action"
                  class="btn btn-sm btn-primary"
                  :disabled="!isConnected"
                >
                  야생 약초 채집 (Gather 🌿)
                </button>
                <button
                  v-else-if="prologueStep === 'P2_OUTPOST_TRAVEL' || prologueStep === 'P2OutpostTravel'"
                  @click="handleP2Action"
                  class="btn btn-sm btn-primary"
                  :disabled="!isConnected"
                >
                  국경 전초기지로 이동 (Travel)
                </button>
                <button
                  v-else-if="prologueStep === 'P3_RESCUE_COMBAT' || prologueStep === 'P3RescueCombat'"
                  @click="handleP3Action"
                  class="btn btn-sm btn-danger"
                  :disabled="!isConnected"
                >
                  고블린 격퇴 & 카엘 구출 (Combat ⚔️)
                </button>
                <button
                  v-else-if="prologueStep === 'P4_SUPPLY_MIGRATION' || prologueStep === 'P4SupplyMigration'"
                  @click="handleP4Action"
                  class="btn btn-sm btn-primary"
                  :disabled="!isConnected"
                >
                  이주 보급품 챙기기 (Pack 📦)
                </button>
                <button
                  v-else-if="prologueStep === 'P5_SHARED_SETTLEMENT' || prologueStep === 'P5SharedSettlement'"
                  @click="handleP5Action"
                  class="btn btn-sm btn-primary"
                  :disabled="!isConnected"
                >
                  공유 정착지 관문 진입
                </button>

                <!-- Trigger Transfer when READY_TO_TRANSFER -->
                <button
                  v-if="prologueState === 'READY_TO_TRANSFER'"
                  @click="handleRequestTransfer"
                  class="btn btn-success btn-block"
                  :disabled="!isConnected"
                >
                  공유 세계로 이관 완료 (World Transfer 🚀)
                </button>

                <!-- Fallback button for simulated failure (TEST-PRO-005) -->
                <button
                  v-if="!prologueFallbackActive && prologueState === 'IN_PROGRESS'"
                  @click="handleSimulateBlockedPath"
                  class="btn btn-secondary btn-xs text-muted"
                  title="Simulate blocked movement for TEST-PRO-005"
                >
                  경로 장애 시뮬레이션
                </button>
              </div>
            </div>
          </div>
        </section>

        <!-- Inventory / Bag Panel (TASK-012A) -->
        <section class="card inventory-card">
          <div class="card-header-flex">
            <h3>인벤토리 (Inventory)</h3>
            <span class="slot-count">{{ inventory.length }} / 16</span>
          </div>
          <div class="inventory-grid">
            <div
              v-for="item in inventory"
              :key="item.id"
              class="inventory-slot"
              @click="handleItemClick(item)"
            >
              <span class="item-icon">{{ item.icon }}</span>
              <span class="item-name">{{ item.name }}</span>
              <span class="item-count">x{{ item.count }}</span>
            </div>
          </div>
        </section>

        <!-- Command Controls -->
        <section class="card actions-card">
          <h3>Quick Commands</h3>
          <div class="action-grid">
            <button @click="sendMoveRelative(0, -10)" class="btn btn-sm">↑ Up</button>
            <div class="horizontal-btns">
              <button @click="sendMoveRelative(-10, 0)" class="btn btn-sm">← Left</button>
              <button @click="sendMoveRelative(10, 0)" class="btn btn-sm">Right →</button>
            </div>
            <button @click="sendMoveRelative(0, 10)" class="btn btn-sm">↓ Down</button>
          </div>

          <div class="action-row">
            <button @click="handleAttackNearest" class="btn btn-warn btn-sm" :disabled="!isConnected">
              ⚔ Attack Nearest
            </button>
            <button @click="handleResync" class="btn btn-secondary btn-sm" :disabled="!isConnected">
              🔄 Force Resync
            </button>
          </div>
        </section>

        <!-- AOI Entities in Vision -->
        <section class="card aoi-card">
          <h3>AOI Entities ({{ entityList.length }})</h3>
          <div class="entity-list">
            <div
              v-for="ent in entityList"
              :key="ent.id"
              class="entity-item"
              :class="{ 'is-self': ent.id === actorId }"
            >
              <div class="entity-badge" :class="(ent.entity_type || 'default').toLowerCase()">
                {{ ent.entity_type || 'Entity' }}
              </div>
              <div class="entity-info">
                <span class="ent-name">{{ ent.name || ent.id }}</span>
                <span class="ent-pos">({{ Math.round(ent.x) }}, {{ Math.round(ent.y) }})</span>
              </div>
              <div class="ent-hp">{{ ent.hp ?? 100 }}/{{ ent.max_hp ?? 100 }}</div>
            </div>
          </div>
        </section>

        <!-- System & Network Log -->
        <section class="card log-card">
          <h3>Event Feed</h3>
          <div class="log-stream" ref="logContainer">
            <div
              v-for="(log, idx) in logs"
              :key="idx"
              class="log-entry"
              :class="log.level"
            >
              <span class="log-time">{{ log.time }}</span>
              <span class="log-text">{{ log.text }}</span>
            </div>
          </div>
        </section>
      </aside>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue';
import Phaser from 'phaser';
import { GameClient } from './network/client.js';
import type { EntityStateWire } from './network/protocol.js';
import { WorldGameScene } from './game/GameScene.js';
import { loadClaimedReceipts, saveClaimedReceipts } from './network/receipt_store.js';

interface LogItem {
  time: string;
  text: string;
  level: 'info' | 'warn' | 'error';
}

interface InventoryItem {
  id: string;
  name: string;
  count: number;
  icon: string;
}

interface CommandFeedback {
  id: string;
  status: 'PENDING' | 'COMMITTED' | 'REJECTED';
  text: string;
  timestamp: number;
}

const DEFAULT_ACCOUNT_ID = '00000000-0000-0000-0000-000000000001';
const DEFAULT_ACTOR_ID = '00000000-0000-0000-0000-000000000001';
const DEFAULT_REGION_ID = '00000000-0000-0000-0000-000000000001';

const defaultWsUrl = typeof window !== 'undefined' && window.location.host
  ? `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`
  : 'ws://localhost:8000/ws';
const wsUrl = ref(defaultWsUrl);
const actorId = ref(DEFAULT_ACTOR_ID);
const connectionStatus = ref<'DISCONNECTED' | 'CONNECTING' | 'AUTHENTICATED' | 'ERROR'>('DISCONNECTED');
const connectionStatusText = ref('DISCONNECTED');
const isConnected = ref(false);
const regionId = ref('');
const currentTick = ref(0);
const currentRev = ref(0);
const rtt = ref(0);

// Wealth & Assets
const gold = ref(150);
const supplyPoints = ref(20);

// Inventory
const inventory = ref<InventoryItem[]>([
  { id: 'herb_01', name: '야생 약초', count: 3, icon: '🌿' },
  { id: 'wood_01', name: '가공 목재', count: 2, icon: '🪵' },
  { id: 'potion_01', name: '회복 물약', count: 5, icon: '🧪' },
  { id: 'sword_01', name: '수습 단검', count: 1, icon: '🗡️' },
]);

// Command Status
const lastCommandFeedback = ref<CommandFeedback | null>(null);

// Dialogue System
const showDialogue = ref(false);
const dialogueNpcName = ref('촌장 엘다');
const dialogueText = ref('개척의 길에 온 것을 환영하네. 정착지로 향하기 전에 주변을 정돈해주게.');
const dialogueChoices = ref<{ label: string; action: () => void }[]>([]);

// Prologue Progression State (TASK-012B)
const prologueState = ref('NOT_STARTED');
const prologueStep = ref('NONE');
const prologueProgress = ref(0);
const prologueTargetCount = ref(0);
const prologueStepTitle = ref('프롤로그: 여정의 시작');
const prologueStepDesc = ref('개인 개척촌(Private Region)에서 여정을 시작하세요.');
const prologueFallbackActive = ref(false);
const prologueFailureReason = ref<string | null>(null);
const isPrologueCompleted = ref(false);

const entities = ref<Map<string, EntityStateWire>>(new Map());
const logs = ref<LogItem[]>([]);
const logContainer = ref<HTMLElement | null>(null);

let client: GameClient | null = null;
let phaserGame: Phaser.Game | null = null;
let gameScene: WorldGameScene | null = null;

const entityList = computed(() => Array.from(entities.value.values()));
const localPlayer = computed(() => entities.value.get(actorId.value));

const hpPercent = computed(() => {
  if (!localPlayer.value) return 0;
  const max = localPlayer.value.max_hp ?? 100;
  const hp = localPlayer.value.hp ?? 100;
  if (max <= 0) return 0;
  return Math.round((hp / max) * 100);
});

function addLog(text: string, level: 'info' | 'warn' | 'error' = 'info') {
  const d = new Date();
  const time = d.toTimeString().split(' ')[0] + '.' + String(d.getMilliseconds()).padStart(3, '0');
  logs.value.push({ time, text, level });
  if (logs.value.length > 100) {
    logs.value.shift();
  }
  nextTick(() => {
    if (logContainer.value) {
      logContainer.value.scrollTop = logContainer.value.scrollHeight;
    }
  });
}

function setCommandStatus(id: string, status: 'PENDING' | 'COMMITTED' | 'REJECTED', text: string) {
  lastCommandFeedback.value = { id, status, text, timestamp: Date.now() };
  if (status !== 'PENDING') {
    setTimeout(() => {
      if (lastCommandFeedback.value?.id === id) {
        lastCommandFeedback.value = null;
      }
    }, 4000);
  }
}

const claimedReceipts = loadClaimedReceipts();

async function fetchWsTicket(account: string, actor: string, region: string): Promise<string> {
  const protocol = typeof window !== 'undefined' && window.location.protocol ? window.location.protocol : 'http:';
  const host = typeof window !== 'undefined' && window.location.host ? window.location.host : 'localhost:8000';
  const res = await fetch(`${protocol}//${host}/api/v1/ws-tickets`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      account_id: account,
      actor_id: actor,
      region_id: region,
    }),
  });
  if (!res.ok) {
    const errText = await res.text();
    throw new Error(`HTTP ${res.status}: ${errText}`);
  }
  const data = await res.json();
  return data.ticket;
}

function ensureUuid(val: string): string {
  const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  if (uuidRegex.test(val)) return val;
  return DEFAULT_ACTOR_ID;
}

function getSafeWsUrl(): string {
  if (typeof window !== 'undefined' && window.location.host) {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${proto}//${window.location.host}/ws`;
  }
  return 'ws://localhost:8000/ws';
}

async function handleConnect() {
  if (connectionStatus.value === 'CONNECTING') {
    return;
  }

  const targetWsUrl = getSafeWsUrl();
  wsUrl.value = targetWsUrl;

  if (client) {
    client.disconnect();
  }

  connectionStatus.value = 'CONNECTING';
  connectionStatusText.value = 'CONNECTING';

  let activeTicket = '';
  try {
    addLog('Requesting fresh one-time auth ticket from server...', 'info');
    const validActor = ensureUuid(actorId.value);
    actorId.value = validActor;
    const validRegion = ensureUuid(regionId.value || DEFAULT_REGION_ID);
    activeTicket = await fetchWsTicket(DEFAULT_ACCOUNT_ID, validActor, validRegion);
    addLog('Auth ticket issued successfully from server.', 'info');
  } catch (err: any) {
    connectionStatus.value = 'ERROR';
    connectionStatusText.value = 'TICKET_ERROR';
    addLog(`Failed to obtain ticket: ${err.message}`, 'error');
    return;
  }

  addLog(`Initiating connection to ${targetWsUrl}...`, 'info');

  client = new GameClient({
    wsUrl: targetWsUrl,
    ticket: activeTicket,
    autoReconnect: false,
  });

  client.on('state_change', (state) => {
    if (state === 'connected') {
      isConnected.value = true;
      connectionStatus.value = 'AUTHENTICATED';
      connectionStatusText.value = 'AUTHENTICATED';
      client?.queryPrologueState();
    } else if (state === 'connecting' || state === 'authenticating') {
      isConnected.value = false;
      connectionStatus.value = 'CONNECTING';
      connectionStatusText.value = state.toUpperCase();
    } else {
      isConnected.value = false;
      connectionStatus.value = 'DISCONNECTED';
      connectionStatusText.value = 'DISCONNECTED';
    }
  });

  client.on('authenticated', (authPayload) => {
    regionId.value = authPayload.region_id;
    if (authPayload.actor_id) {
      actorId.value = authPayload.actor_id;
      if (gameScene) {
        gameScene.setLocalActorId(authPayload.actor_id);
      }
    }
    addLog(`Authenticated session ${authPayload.session_id} in region ${authPayload.region_id}`, 'info');
  });

  client.on('auth_error', (payload) => {
    connectionStatus.value = 'ERROR';
    connectionStatusText.value = 'AUTH_ERROR';
    addLog(`Auth failed: [${payload.code}] ${payload.message}`, 'error');
  });

  client.on('snapshot', (snap) => {
    currentRev.value = snap.world_revision;
    currentTick.value = snap.region_tick;
    addLog(`Received snapshot rev=${snap.world_revision} entities=${snap.entities.length}`, 'info');
    updateGameScene();
  });

  client.on('delta', (delta) => {
    currentTick.value = delta.region_tick;
    updateGameScene();
  });

  client.on('entity_enter', (ent: EntityStateWire) => {
    addLog(`Entity entered AOI: ${ent.name || ent.id} (${ent.entity_type})`, 'info');
    updateGameScene();
  });

  client.on('entity_leave', (entId: string) => {
    addLog(`Entity left AOI: ${entId}`, 'info');
    updateGameScene();
  });

  client.on('command_status', (ack) => {
    const isSuccess = ack.status === 'committed' || ack.status === 'accepted';
    addLog(`Command ${ack.command_id.slice(0, 8)} status=${ack.status}`, isSuccess ? 'info' : 'warn');
    setCommandStatus(
      ack.command_id,
      ack.status === 'committed' ? 'COMMITTED' : ack.status === 'rejected' ? 'REJECTED' : 'PENDING',
      `명령 ${ack.command_id.slice(0, 8)}: ${ack.status.toUpperCase()}${ack.reason ? ` (${ack.reason})` : ''}`,
    );
  });

  client.on('pong', ({ rtt: latency }) => {
    rtt.value = latency;
  });

  client.on('prologue_state_sync', (sync) => {
    prologueState.value = sync.state;
    prologueStep.value = sync.step;
    prologueProgress.value = sync.progress;
    prologueTargetCount.value = sync.target_count;
    prologueStepTitle.value = sync.step_title;
    prologueStepDesc.value = sync.step_description;
    prologueFallbackActive.value = sync.fallback_active;
    prologueFailureReason.value = sync.last_failure_reason || null;
    isPrologueCompleted.value = sync.is_completed;
    addLog(`Prologue sync: state=${sync.state} step=${sync.step} completed=${sync.is_completed}`, 'info');
  });

  client.on('prologue_transfer_result', (res) => {
    addLog(`Prologue transfer confirmed: id=${res.transfer_id} status=${res.status}`, 'info');
    if (res.rewards && res.rewards.length > 0) {
      let newlyClaimed = 0;
      for (const reward of res.rewards) {
        if (claimedReceipts.has(reward.receipt_id)) {
          continue;
        }
        claimedReceipts.add(reward.receipt_id);
        newlyClaimed += 1;
        if (reward.sku === 'gold' || reward.sku === 'currency') {
          gold.value += reward.quantity;
        } else if (reward.sku === 'supply_pack' || reward.sku === 'supplies') {
          supplyPoints.value += reward.quantity;
        } else {
          inventory.value.push({
            id: reward.sku,
            name: reward.sku,
            count: reward.quantity,
            icon: '📦',
          });
        }
      }
      if (newlyClaimed > 0) {
        saveClaimedReceipts(claimedReceipts);
        addLog(`Authoritative rewards claimed (${newlyClaimed} receipts applied).`, 'info');
      } else {
        addLog(`Transfer result replayed idempotently (no duplicate rewards applied).`, 'info');
      }
    }
  });

  client.on('error', (err) => {
    addLog(`Socket error: ${err}`, 'error');
  });

  client.connect();
}

function handleDisconnect() {
  if (client) {
    client.disconnect();
    client = null;
  }
  entities.value.clear();
  isConnected.value = false;
  connectionStatus.value = 'DISCONNECTED';
  connectionStatusText.value = 'DISCONNECTED';
  addLog('Disconnected from server.', 'info');
  updateGameScene();
}

function updateGameScene() {
  if (client && gameScene) {
    const map = new Map<string, EntityStateWire>();
    for (const ent of client.getEntities()) {
      map.set(ent.id, ent);
    }
    entities.value = map;
    gameScene.updateEntities(entities.value, actorId.value);
  }
}

function sendMoveRelative(dx: number, dy: number) {
  if (!client || !localPlayer.value) return;
  const targetX = localPlayer.value.x + dx;
  const targetY = localPlayer.value.y + dy;
  const cmdId = client.sendMove(targetX, targetY);
  if (cmdId) {
    setCommandStatus(cmdId, 'PENDING', `이동 중: (${Math.round(targetX)}, ${Math.round(targetY)})`);
    addLog(`Sent move to (${Math.round(targetX)}, ${Math.round(targetY)})`, 'info');
  }
}

function handleAttackNearest() {
  if (!client || !localPlayer.value) return;
  let nearestId: string | null = null;
  let nearestDist = Infinity;

  for (const ent of entityList.value) {
    if (ent.id === actorId.value) continue;
    const dist = Math.hypot(ent.x - localPlayer.value.x, ent.y - localPlayer.value.y);
    if (dist < nearestDist) {
      nearestDist = dist;
      nearestId = ent.id;
    }
  }

  if (nearestId) {
    const cmdId = client.sendAttack(nearestId);
    if (cmdId) {
      setCommandStatus(cmdId, 'PENDING', `공격 명령: 대상 ${nearestId}`);
      addLog(`Sent attack on nearest entity ${nearestId}`, 'info');
      if (gameScene) {
        const ent = entities.value.get(nearestId);
        if (ent) {
          gameScene.showDamageText(ent.x, ent.y, Math.floor(Math.random() * 20) + 15);
          gameScene.triggerHitEffect(nearestId);
        }
      }
    }
  } else {
    addLog('No target nearby to attack', 'warn');
  }
}

function handleResync() {
  if (client) {
    client.sendResyncRequest('user_manual_resync');
    addLog('Requested full world resync', 'info');
  }
}

// Prologue UI Handlers (TASK-012B)
function handleStartPrologue() {
  if (!client) return;
  client.startPrologue();
  addLog('Sent PrologueStart command', 'info');
}

function handleP0Action() {
  showDialogue.value = true;
  dialogueNpcName.value = '촌장 엘다';
  dialogueText.value = '개척지에 잘 도착했네. 이 땅에서 살아남으려면 먼저 기본 생존 자원을 채집해야 하네.';
  dialogueChoices.value = [
    {
      label: '말씀대로 하겠습니다. (수락)',
      action: () => {
        closeDialogue();
        client?.sendPrologueStepAction('talk_to_npc', 'Elder Elda');
        addLog('Completed P0: Talked with Elder Elda', 'info');
      },
    },
  ];
}

function handleP1Action() {
  client?.sendPrologueStepAction('gather', 'wild_herbs');
  inventory.value.push({
    id: `herb_${Date.now()}`,
    name: '야생 약초',
    count: 1,
    icon: '🌿',
  });
  addLog('Gathered wild herbs (P1 progress)', 'info');
}

function handleP2Action() {
  client?.sendPrologueStepAction('waypoint', 'frontier_outpost');
  addLog('Reached frontier outpost waypoint (P2 complete)', 'info');
}

function handleP3Action() {
  client?.sendPrologueStepAction('defeat_encounter', 'goblin_scout');
  if (localPlayer.value && gameScene) {
    gameScene.showDamageText(localPlayer.value.x + 30, localPlayer.value.y, 45, true);
  }
  addLog('Defeated goblin scout and rescued Kael! (P3 complete)', 'info');
}

function handleP4Action() {
  client?.sendPrologueStepAction('pack_supplies', 'frontier_cart_supplies');
  supplyPoints.value += 30;
  addLog('Packed frontier cart supplies (P4 complete)', 'info');
}

function handleP5Action() {
  client?.sendPrologueStepAction('enter_settlement', 'settlement_gate');
  addLog('Arrived at settlement gate (P5 complete - Ready To Transfer)', 'info');
}

function handleRequestTransfer() {
  if (!client) return;
  client.requestPrologueTransfer();
  addLog('Requested world transfer to shared settlement', 'info');
}

function handleSimulateBlockedPath() {
  if (!client) return;
  client.triggerPrologueFallback('BlockedPath: Frontier bridge destroyed by rockslide');
  addLog('Simulated blocked path failure (TEST-PRO-005 trigger)', 'warn');
}

function handleUseFallback() {
  if (!client) return;
  client.sendPrologueStepAction('talk_to_npc', 'Emergency Guide');
  addLog('Utilized emergency fallback pathway to continue prologue (TEST-PRO-006)', 'info');
}

function closeDialogue() {
  showDialogue.value = false;
}

function handleItemClick(item: InventoryItem) {
  addLog(`Item selected: ${item.name} (Count: ${item.count})`, 'info');
}

onMounted(() => {
  const container = document.getElementById('phaser-container');
  if (!container) return;

  gameScene = new WorldGameScene();

  const config: Phaser.Types.Core.GameConfig = {
    type: Phaser.AUTO,
    parent: 'phaser-container',
    width: container.clientWidth || 800,
    height: container.clientHeight || 600,
    backgroundColor: '#0d1117',
    scene: [gameScene],
  };

  phaserGame = new Phaser.Game(config);

  gameScene.init({
    localActorId: actorId.value,
    callbacks: {
      onTileClick: (worldX, worldY) => {
        if (!client || !isConnected.value) return;
        const cmdId = client.sendMove(worldX, worldY);
        if (cmdId) {
          setCommandStatus(cmdId, 'PENDING', `이동 중: (${worldX}, ${worldY})`);
          addLog(`Clicked move target: (${worldX}, ${worldY})`, 'info');
        }
      },
      onEntityClick: (entityId, entityType) => {
        if (!client || !isConnected.value) return;
        const typeStr = (entityType || '').toLowerCase();
        if (typeStr.includes('monster') || typeStr.includes('mob')) {
          client.sendAttack(entityId);
          addLog(`Targeted attack on ${entityId}`, 'info');
        } else if (typeStr.includes('npc')) {
          showDialogue.value = true;
          dialogueNpcName.value = entityId;
          dialogueText.value = `안녕하신가, 방랑자여. 이 땅은 위험하지만 기회가 가득하다네.`;
          dialogueChoices.value = [
            {
              label: '도움이 필요합니다.',
              action: () => closeDialogue(),
            },
          ];
        }
      },
    },
  });
});

onUnmounted(() => {
  if (client) {
    client.disconnect();
  }
  if (phaserGame) {
    phaserGame.destroy(true);
  }
});
</script>

<style scoped>
.game-wrapper {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  background-color: #0d1117;
  color: #c9d1d9;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
  overflow: hidden;
}

.top-hud {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 16px;
  background-color: #161b22;
  border-bottom: 1px solid #30363d;
  height: 48px;
  box-sizing: border-box;
}

.hud-section {
  display: flex;
  align-items: center;
  gap: 12px;
}

.logo {
  font-size: 16px;
  font-weight: 700;
  margin: 0;
  color: #f0f6fc;
}

.badge {
  font-size: 11px;
  font-weight: 600;
  background-color: #238636;
  color: #ffffff;
  padding: 2px 6px;
  border-radius: 10px;
}

.conn-status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 12px;
  background-color: #21262d;
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background-color: #8b949e;
}

.conn-status.authenticated .status-dot {
  background-color: #2ea043;
}
.conn-status.connecting .status-dot {
  background-color: #d29922;
}
.conn-status.error .status-dot {
  background-color: #cf222e;
}

.stats {
  display: flex;
  gap: 10px;
  font-size: 12px;
  color: #8b949e;
}

.stat-item strong {
  color: #c9d1d9;
}

.wealth-assets {
  display: flex;
  gap: 8px;
}

.asset-badge {
  display: flex;
  align-items: center;
  gap: 4px;
  background: #21262d;
  border: 1px solid #30363d;
  border-radius: 6px;
  padding: 3px 8px;
  font-size: 12px;
  font-weight: 600;
}

.asset-badge.gold {
  color: #e3b341;
  border-color: #d29922;
}

.asset-badge.supplies {
  color: #58a6ff;
  border-color: #1f6feb;
}

.auth-controls {
  display: flex;
  gap: 6px;
}

.input {
  background-color: #0d1117;
  border: 1px solid #30363d;
  color: #c9d1d9;
  padding: 4px 8px;
  border-radius: 4px;
  font-size: 12px;
  width: 130px;
}

.input:focus {
  outline: none;
  border-color: #58a6ff;
}

.btn {
  background-color: #21262d;
  border: 1px solid #30363d;
  color: #c9d1d9;
  padding: 4px 10px;
  border-radius: 4px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: 0.15s ease-in-out;
}

.btn:hover:not(:disabled) {
  background-color: #30363d;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-primary {
  background-color: #238636;
  border-color: #2ea043;
  color: #ffffff;
}
.btn-primary:hover:not(:disabled) {
  background-color: #2ea043;
}

.btn-danger {
  background-color: #da3633;
  border-color: #f85149;
  color: #ffffff;
}

.btn-warn {
  background-color: #9e6a03;
  border-color: #d29922;
  color: #ffffff;
}

.btn-success {
  background-color: #238636;
  border-color: #3fb950;
  color: #ffffff;
  font-weight: 700;
}

.btn-secondary {
  background-color: #30363d;
}

.btn-sm {
  padding: 3px 8px;
  font-size: 11px;
}

.btn-xs {
  padding: 2px 6px;
  font-size: 10px;
}

.btn-block {
  width: 100%;
}

.main-content {
  display: flex;
  flex: 1;
  position: relative;
  overflow: hidden;
}

.canvas-wrapper {
  position: relative;
  flex: 1;
  height: 100%;
}

.canvas-container {
  width: 100%;
  height: 100%;
}

/* Command Status Floating Toast */
.command-status-badge {
  position: absolute;
  top: 16px;
  left: 16px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  background: rgba(22, 27, 34, 0.9);
  backdrop-filter: blur(4px);
  border: 1px solid #30363d;
  z-index: 10;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
}

.command-status-badge.pending {
  border-color: #d29922;
  color: #e3b341;
}
.command-status-badge.pending .status-dot {
  background-color: #d29922;
  animation: pulse 1s infinite alternate;
}

.command-status-badge.committed {
  border-color: #2ea043;
  color: #3fb950;
}
.command-status-badge.committed .status-dot {
  background-color: #2ea043;
}

.command-status-badge.rejected {
  border-color: #cf222e;
  color: #f85149;
}
.command-status-badge.rejected .status-dot {
  background-color: #cf222e;
}

@keyframes pulse {
  from { opacity: 0.4; }
  to { opacity: 1; }
}

/* Dialogue Box Overlay */
.dialogue-overlay {
  position: absolute;
  bottom: 24px;
  left: 50%;
  transform: translateX(-50%);
  width: 85%;
  max-width: 650px;
  z-index: 20;
}

.dialogue-box {
  background: rgba(22, 27, 34, 0.95);
  backdrop-filter: blur(8px);
  border: 2px solid #58a6ff;
  border-radius: 8px;
  padding: 16px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
}

.dialogue-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.dialogue-avatar {
  font-size: 18px;
}

.dialogue-speaker {
  font-size: 14px;
  color: #58a6ff;
}

.dialogue-body {
  font-size: 13px;
  line-height: 1.5;
  color: #f0f6fc;
  margin: 0 0 12px 0;
}

.dialogue-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

/* Sidebar */
.sidebar-panel {
  width: 340px;
  background-color: #161b22;
  border-left: 1px solid #30363d;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px;
  overflow-y: auto;
  box-sizing: border-box;
}

.card {
  background-color: #0d1117;
  border: 1px solid #30363d;
  border-radius: 6px;
  padding: 10px;
}

.card h3 {
  font-size: 13px;
  font-weight: 600;
  margin: 0 0 8px 0;
  color: #8b949e;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.card-header-flex {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.slot-count {
  font-size: 11px;
  color: #8b949e;
}

/* Prologue Card */
.prologue-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.prologue-status-tag {
  font-size: 10px;
  font-weight: 700;
  padding: 2px 6px;
  border-radius: 4px;
  background: #30363d;
  color: #c9d1d9;
}

.prologue-status-tag.in_progress {
  background: #1f6feb;
  color: #ffffff;
}

.prologue-status-tag.ready_to_transfer {
  background: #238636;
  color: #ffffff;
}

.prologue-status-tag.completed {
  background: #2ea043;
  color: #ffffff;
}

.prologue-completed-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  background: rgba(46, 160, 67, 0.15);
  border: 1px solid #2ea043;
  border-radius: 6px;
  padding: 10px;
  color: #3fb950;
  font-size: 12px;
}

.prologue-completed-banner .check-icon {
  font-size: 20px;
  font-weight: bold;
}

.prologue-completed-banner p {
  margin: 2px 0 0 0;
  font-size: 11px;
  color: #8b949e;
}

.step-title-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 4px;
}

.step-title {
  font-size: 12px;
  color: #f0f6fc;
}

.step-badge {
  font-size: 10px;
  background: #21262d;
  border: 1px solid #30363d;
  padding: 1px 4px;
  border-radius: 4px;
  color: #58a6ff;
}

.step-desc {
  font-size: 11px;
  color: #8b949e;
  margin: 0 0 8px 0;
  line-height: 1.4;
}

.progress-container {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.progress-bar-bg {
  flex: 1;
  height: 6px;
  background: #21262d;
  border-radius: 3px;
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  background: #2ea043;
  transition: width 0.3s ease;
}

.progress-label {
  font-size: 10px;
  color: #8b949e;
  font-family: monospace;
}

.prologue-fallback-alert {
  display: flex;
  gap: 8px;
  background: rgba(210, 153, 34, 0.15);
  border: 1px solid #d29922;
  border-radius: 4px;
  padding: 8px;
  margin-bottom: 8px;
  font-size: 11px;
}

.prologue-fallback-alert .alert-icon {
  font-size: 14px;
}

.prologue-fallback-alert p {
  margin: 2px 0 6px 0;
  color: #e3b341;
}

.prologue-actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* Inventory Grid */
.inventory-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 6px;
}

.inventory-slot {
  display: flex;
  align-items: center;
  gap: 6px;
  background: #161b22;
  border: 1px solid #30363d;
  border-radius: 4px;
  padding: 6px;
  cursor: pointer;
  transition: 0.15s ease;
}

.inventory-slot:hover {
  background: #21262d;
  border-color: #58a6ff;
}

.item-icon {
  font-size: 16px;
}

.item-name {
  flex: 1;
  font-size: 11px;
  color: #c9d1d9;
}

.item-count {
  font-size: 10px;
  color: #8b949e;
  font-family: monospace;
}

/* Player stats */
.player-stats {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.stat-row {
  display: flex;
  align-items: center;
  font-size: 12px;
}

.lbl {
  width: 40px;
  color: #8b949e;
}

.val {
  color: #f0f6fc;
}

.hp-gauge {
  flex: 1;
  position: relative;
  height: 16px;
  background-color: #21262d;
  border-radius: 3px;
  overflow: hidden;
}

.hp-fill {
  height: 100%;
  background-color: #2ea043;
  transition: width 0.2s ease-out;
}

.hp-text {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 600;
  color: #ffffff;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.8);
}

.empty-hint {
  font-size: 11px;
  color: #8b949e;
  font-style: italic;
}

/* Action Controls */
.action-grid {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  margin-bottom: 8px;
}

.horizontal-btns {
  display: flex;
  gap: 8px;
}

.action-row {
  display: flex;
  gap: 6px;
  justify-content: stretch;
}

.action-row .btn {
  flex: 1;
}

/* AOI Entity list */
.entity-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 120px;
  overflow-y: auto;
}

.entity-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px;
  border-radius: 4px;
  background-color: #161b22;
  font-size: 11px;
}

.entity-item.is-self {
  border: 1px solid #2ea043;
}

.entity-badge {
  font-size: 9px;
  padding: 1px 4px;
  border-radius: 3px;
  background-color: #30363d;
  color: #8b949e;
}

.entity-badge.monster,
.entity-badge.mob {
  background-color: #da3633;
  color: #ffffff;
}

.entity-badge.npc,
.entity-badge.merchant {
  background-color: #1f6feb;
  color: #ffffff;
}

.entity-info {
  flex: 1;
  display: flex;
  gap: 4px;
}

.ent-pos {
  color: #8b949e;
  font-family: monospace;
}

.ent-hp {
  color: #3fb950;
  font-weight: 600;
}

/* Log card */
.log-stream {
  height: 120px;
  overflow-y: auto;
  background-color: #161b22;
  border-radius: 4px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-family: monospace;
  font-size: 11px;
}

.log-entry {
  display: flex;
  gap: 6px;
}

.log-time {
  color: #8b949e;
}

.log-entry.info .log-text {
  color: #c9d1d9;
}
.log-entry.warn .log-text {
  color: #d29922;
}
.log-entry.error .log-text {
  color: #cf222e;
}

.font-mono {
  font-family: monospace;
}
</style>
