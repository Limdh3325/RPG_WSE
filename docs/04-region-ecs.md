# 04. Region Worker · ECS · Ownership (LLD)

## 1. Region 소유권 모델

- 각 Region은 정확히 **하나의 Active Owner**만 갖는다.
- `region_leases(region_id, owner_id, ownership_epoch, lease_expires_at)`를 PostgreSQL에 관리.
- 모든 **durable DB 쓰기 트랜잭션**은 lease row를 잠그고 `epoch = expected_epoch` 및 `lease_expires_at > clock_timestamp()` 검사 후 진행한다.
- GC/스케줄 지연 때문에 이전 Worker가 살아있어도 새로운 Epoch를 부여하면 이전 Worker의 Commit은 거부된다.
- Gateway의 Region routing cache는 epoch와 함께 사용하며 불일치 시 refresh 후 명령 다시 라우팅.

## 2. ECS World 구조

```
RegionRuntime
  ├── RegionMetadata { region_id, epoch, status, tick }
  ├── bevy_ecs::World
  │   ├── Entity(opaque Bevy handle) -> StableEntityId(UUID)
  │   ├── Position, Velocity, Navigation, Collider
  │   ├── Health, CombatState, InventoryProjection
  │   ├── Needs, Personality, AIState, MemoryRef
  │   └── Resources: Clock, MapGrid, SpatialIndex, Rng
  ├── Schedule (ordered systems)
  ├── stable_entity_index: UUID -> Bevy Entity
  ├── command_queue / priority_ack_queue
  ├── pending_durable: command_id -> reservations
  └── changed_entity_set / tick_metrics
```

Bevy `Entity`는 Region의 재시작 시 달라질 수 있으므로 네트워크·DB·로그에 직접 저장 금지. `StableEntityId(Uuid)`를 사용하고 복구 시 인덱스를 다시 만든다.

## 3. 필수 Components

| Component | 필드(예시) | 검증 규칙 |
|---|---|---|
| StableId | UUID | Region 내 unique |
| Position | `x_millitile:i64, y_millitile:i64` | 유효 지형·영역 좌표 |
| Velocity | `dx,dy:i32` | 이동 속도/능력 상한 |
| Navigation | target, path, path_version, state | 경로 유효/상태 머신 |
| Health | current,max:i32 | 0 ≤ current ≤ max |
| Needs | hunger,fatigue,safety:u16 | 0..1000 clamp |
| Personality | courage,greed,loyalty:i16 | -1000..1000 |
| AIState | goal,action,next_decision_tick | 유효 계획/쿨다운 |
| InventoryProjection | committed_version, slots | DB Commit 결과만 반영 |
| FactionMembership | faction_uuid,standing | GlobalWorker의 버전 검증 |

## 4. Region Worker Thread

```rust
struct RegionHandle {
    id: uuid::Uuid,
    epoch: u64,
    cmd_tx: tokio::sync::mpsc::Sender<RegionMessage>,
}

enum RegionMessage {
    Command(CommandEnvelope),
    PersistenceAck(PersistenceAck),
    GlobalStateUpdate(GlobalProjection),
    SetSimulationMode(SimulationMode),
    Shutdown,
}
```

Tokio 네트워크는 `Sender::try_send`/`send`를 상황에 맞춰 사용; Region 전용 OS 스레드는 `Receiver::try_recv`로 bounded 입력을 매 Tick 전 수집한다. `try_send` 성공은 **큐에 들어갔다는 의미**이지 성공 실행/DB 확정이 아니다.

### 큐 정책

| 큐 | 임시 상한(가정) | 가득 찼을 때 |
|---|---:|---|
| Player Command / Region | 4096 | `SERVER_BUSY` 반환; 세션별 rate limit |
| Persistence Ack | 1024 | 우선순위 보장/별도 경로; 안전 중단 |
| Transient Delta | 2048 | 위치 delta 병합/old delta 폐기 후 snapshot |
| Durable Intent | 512 | 신규 durable 명령 거부, in-flight 예약 유지 |
| Control | 64 | 우선 서비스; 불가 시 health fault |

## 5. Region Lifecycle

`LOADING → ACTIVE/BACKGROUND → PAUSED/MIGRATING → STOPPED`. 별도 `health = HEALTHY/DEGRADED/FAILED` 유지(상태와 건강을 섞지 않음).

- LOADING: Epoch 취득 → Snapshot·이벤트·영속 Projection 일치 검증 → Spatial index 재구성.
- ACTIVE: 플레이어가 있는 영역, 상세 이동/전투/AI.
- BACKGROUND: 실시간 개별 NPC 이동의 일부 생략, 생산/소비 집계; 중요 NPC 독립 상태는 보존.
- MIGRATING: 신규 Command admission 정지, durable drain/확정, 안전 checkpoint, Epoch 양도, 대상 Worker 로드 후 라우팅 변경.
- PAUSED: 관리/이상 상태; 외부 입력 명확히 거부 또는 대기.

## 6. Region 간 이동 (MVP 단일 서버)

```mermaid
sequenceDiagram
  participant S as Source Region
  participant C as Transfer Coordinator
  participant D as Durable Store
  participant T as Target Region
  S->>C: TransferRequest(entity, source_epoch)
  C->>S: Mark TRANSFERRING; stop local commands
  C->>D: transfer record unique transfer_id / fence epochs
  D-->>C: committed transfer intent
  C->>T: spawn frozen entity snapshot
  T-->>C: target ready
  C->>D: commit new region_id/location
  D-->>C: committed
  C->>S: despawn source
  C->>T: activate entity
```

실패 시 `transfer_id` receipt로 재시작: 목적지 중복 생성은 idempotent upsert, 소스/목적지 동시 활성화 금지. 구현 첫 단계에서는 `Player` 이전만 지원, 임의 NPC 이동은 후속 확대.

## 7. Global State 원칙

- Kingdom/Faction/World Clock/거시경제 등 공유 상태에는 별도 GlobalWorker single writer 적용.
- Region은 `global_projection_version`을 포함한 읽기 전용 Snapshot을 사용.
- 글로벌 명령은 GlobalWorker로 전달하고, 결과 Event를 Region에 비동기 적용.
- 일관성이 필요한 다중 지역 거래는 MVP 제외; 최종 일관성만 허용하는 이벤트는 버전·중복 처리 필수.

## 8. 테스트

- TEST-REGION-001: 동일 Region Epoch를 가진 Worker 2개 중 하나만 Commit 성공.
- TEST-REGION-002: Worker panic 후 새로운 Epoch로 복구, 이전 Worker 쓰기 거부.
- TEST-REGION-003: 2개 Region 동시 실행에서 ECS 상태 교차 수정 없음.
- TEST-REGION-004: BACKGROUND↔ACTIVE 전환 후 NPC/재고 합계 불변식 유지.
- TEST-REGION-005: transfer 중 임의 단계 종료 시 캐릭터 중복 0.
