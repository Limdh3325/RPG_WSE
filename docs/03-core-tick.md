# 03. 시뮬레이션 코어와 Fixed Tick (LLD)

## 1. 실행 계약

- `tick_rate_hz = 10`, `fixed_delta_ms = 100` (**[가정]**). 게임 시간 배율(`game_seconds_per_real_second=10`)은 별도 설정.
- `tick = n`의 상태 변경은 **명시적으로 정해진 순서**로만 수행한다.
- 시뮬레이션 계산에는 `Instant::elapsed()`/벽시계를 직접 사용하지 않는다. 타임아웃/운영 감시용으로만 허용.
- `WorldState`는 논리적 개념. 실제로는 각 Region에 `bevy_ecs::World` 한 개, 전역 상태는 GlobalWorker가 소유.
- 계산 순서의 결과는 무관한 해시맵 순서에 의존하지 않는다. stable UUID 정렬 또는 의도적으로 순서를 기록한 입력을 사용한다.

## 2. Tick 단계

| 순서 | 단계 | 입력 | 출력/불변식 |
|---|---|---|---|
| 0 | Admission | Gateway Commands | 적용 Tick·RegionSeq 확정, 초과/중복 거부 |
| 1 | Command Validation | Ready queue | Pending action / Rejected |
| 2 | Perception | Spatial index | 주변 가시 정보 / 위험 신호 |
| 3 | Navigation & Movement | 목표/속도/타일 맵 | 위치·충돌 결과 |
| 4 | Combat | 공격 입력/거리/HP | 피해·사망·보상 intent |
| 5 | NPC Decisions | 대상 NPC 스케줄 | 다음 Tick 실행 Command |
| 6 | World/Economy | Resource / Settlement | 생산·소비·가격·환경 |
| 7 | Event Rules | State changes/events | 결정적인 후속 명령 예약 |
| 8 | Finalize | transient changes, staged durable | delta, durable requests, trace |
| 9 | Output | Event/Delta queues | Replicator/Writer에 bounded enqueue |

중요: 영속 트랜잭션 ACK는 각 Tick의 **단계 0 전** 우선 처리하며, 영속 intent의 원래 주문 순서를 보존한다. ACK가 재정렬될 수 있는 경우 actor/item 단위 in-flight 직렬화로 일관성을 지킨다.

## 3. 핵심 Rust 인터페이스 (설계 계약)

```rust
pub struct TickContext {
    pub tick: u64,
    pub fixed_delta_ms: u32,
    pub game_time_ms: u64,
    pub seed: [u8; 32],
}

pub struct RegionRuntime {
    pub world: bevy_ecs::world::World,
    pub schedule: bevy_ecs::schedule::Schedule,
    pub metadata: RegionMetadata,
    // command inbox, pending transactions, outbox buffers,
    // stable_id <-> ECS Entity index, metrics
}

pub struct TickOutcome {
    pub tick: u64,
    pub transient_deltas: Vec<StateChange>,
    pub durable_intents: Vec<DurableIntent>,
    pub generated_commands: Vec<CommandEnvelope>,
}

pub trait HeadlessEngine {
    fn enqueue(&mut self, command: CommandEnvelope) -> Result<(), EngineError>;
    fn step(&mut self) -> Result<TickOutcome, EngineError>;
    fn snapshot_committed(&self) -> Result<RegionCheckpoint, EngineError>;
}
```

`TickOutcome`은 **아직 DB에서 확정되지 않은** durable intent를 포함할 수 있다. Replicator는 transient delta만 즉시 사용하며, durable result는 `PersistenceAck` 이후 별도로 내보낸다.

## 4. Bevy Schedule 규칙

```rust
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimPhase {
    Perception, Movement, Combat, NpcDecisions, World, EventRules,
}
// schedule.configure_sets((...phase...).chain());
// schedule.set_executor(SingleThreadedExecutor::default());
```

- 시스템은 component query와 리소스만 사용, IO 없음.
- 각 System은 입력 상태와 TickContext에서만 계산. `Commands` deferred 적용 지점을 명시.
- NPC가 새 결정을 내리면 **다음 Tick** 명령에 등록한다. 피해·이동 중단처럼 같은 Tick 내 필수 반응은 Combat/Movement 단계에서만 수행.
- 초기에는 single-thread executor. 이후 병렬화할 때 golden replay 통과 필수.

## 5. NPC 분산 스케줄링

- `decision_interval_ticks=10`이면 `hash(stable_id) % 10 == tick % 10`인 NPC만 일반 판단.
- 근접 공격·피해 등 인터럽트 반응은 매 Tick 조회 가능한 별도 urgent queue.
- 장기 GOAP plan은 100 Tick 주기 또는 목표 무효화 시 실행.
- budget: `max_npc_evaluations_per_tick`, `max_path_expansions_per_tick`; 처리 못한 저우선순위 작업은 뒤로 미루되 starvation 방지용 aging.

## 6. RNG / 결정성

- RNG는 엔진 버전이 고정된 알고리즘(예: ChaCha8 계열 고정 버전) 또는 `(world_seed, region_uuid, tick, system_id, actor_uuid, counter)`를 안정된 해시로 조합한 stream으로 구성.
- `HashMap` 순회 순서, OS thread scheduling, float roundoff에 의존한 분기 금지.
- 좌표/재화/피해는 integer/fixed-point 추천(`i64` 밀리타일, 금화 정수).
- 비동기 LLM 결과는 **외부 Command**로 변환해 적용 Tick과 payload를 로그에 남긴다.
- 결정성 테스트는 `initial snapshot + engine/rules version + ordered input log + RNG state`로 출력 해시 검증.

## 7. Tick Scheduler 알고리즘 (의사코드)

```rust
let step = Duration::from_millis(100);
let mut deadline = Instant::now();
while running {
    drain_priority_acks_and_controls();
    collect_commands_with_admission_limit();
    let out = engine.step()?; // never await
    try_enqueue_transient(out.transient_deltas); // coalesce if full
    enqueue_or_reject_durable_safely(out.durable_intents)?;
    emit_metrics();
    deadline += step;
    if Instant::now() < deadline { sleep_until(deadline); }
    else {
        record_overrun();
        // 정책: 중간 상태를 생략하지 않되, 저우선 작업을 지연,
        // 지속 초과 시 admission 제한 / degraded 진입
    }
}
```

**보장 차이:** 동일 입력 시 논리 Tick 결과 재현은 목표이나, 실제 수신 시간·네트워크 지연은 재현 불가. 라이브 모드에서는 `assigned_tick`을 로그로 남긴다.

## 8. Tick 실패 정책

| 분류 | 처리 |
|---|---|
| 명령 검증 실패 | `REJECTED`, Tick 지속 |
| 계산 중 데이터 불변식 위반 | Region `PAUSED`, 증거 snapshot/log, 운영 알림 |
| DB Writer 단절 | 신규 durable intent 차단, Tick transient 제한적 지속 |
| 5회 연속 Tick 초과 | `DEGRADED`, AI 낮은 우선순위 감축 |
| 스레드 panic | Region lease 안전 해제/만료 후 복구 |

## 9. 테스트

- TEST-TICK-001: 10,000 Tick을 headless로 실행해 동일 seed/inputs에서 상태 해시가 같다.
- TEST-TICK-002: NPC별 예정 Tick 실행 횟수가 정확하다.
- TEST-TICK-003: 입력 순서가 다르면 로그상 정렬·assign 순서대로 결과가 재현된다.
- TEST-TICK-004: 초과 Tick 발생 시 tick number가 생략되지 않는다.
- TEST-TICK-005: 비동기 LLM 지연 10초가 Tick 실행 시간을 늘리지 않는다.
