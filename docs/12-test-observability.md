# 12. 테스트 전략 · 벤치마크 · 관측성

**한국어** | [English](en/12-test-observability.md)

## 1. 품질 게이트

모든 TASK는 `unit → integration → deterministic → recovery → load` 중 해당 범위를 통과해야 한다. 실행하지 않은 테스트는 `NOT RUN`으로 명시하며, 예상 성능 수치를 실측치로 표현하지 않는다.

## 2. 테스트 층위

| 계층 | 테스트 대상 | 도구 예시 |
|---|---|---|
| Unit | damage, AI scoring, pathfinding, utility | Rust `cargo test` |
| Property | 자원 보존, 정렬/경계값, 불변식 | proptest |
| Deterministic | 동일 입력·seed·버전의 snapshot hash | headless runner |
| Integration | Command→Region→Writer→DB→ACK | Docker Testcontainers 또는 임시 PG |
| Fault | crash/ACK 유실/DB disconnect/lease fencing | failpoint + 프로세스 중단 |
| Protocol | schema/codegen/WS state machine | client/server fixture |
| Soak | 24h~72h 시뮬레이션 | benchmark runner |
| Load | 1k/5k/10k NPC, 10/100/500 sessions | k6/커스텀 WS load generator |

## 3. 결정성 검증

### Golden Test Fixture

```yaml
fixture_id: village_food_shortage_v1
engine_version: 0.1.0
rules_version: 1
world_seed: 12345
region_id: 8231eeec-2bf0-4a87-8671-d61016828889
initial_snapshot: fixtures/village_initial.bin
inputs: fixtures/village_inputs.jsonl  # assigned_tick + ingress_seq 포함
run_ticks: 10000
expected_state_hash: "<record-after-first-validated-run>"
expected_event_hash: "<record-after-first-validated-run>"
```

`expected_*`를 실제 실행/검토 없이 작성하면 안 된다. 최초 기준선은 승인된 빌드 결과를 `golden`으로 저장하고 변경 이유/리뷰를 기록.

### 테스트 절차

1. 최신 snapshot + RNG + 규칙 버전 로드.
2. 입력 로그를 `(assigned_tick, ingress_seq)` 순서로 주입.
3. 10,000 Tick 실행.
4. stable UUID로 정렬·정규화된 상태를 직렬화하고 SHA-256 hash.
5. 도메인 이벤트를 `(region_event_seq,event_type,payload)` 기준 정규화 후 hash.
6. baseline과 비교; 다른 경우 최초 divergence tick 및 entity/event ID 출력.

## 4. 장애 주입 매트릭스

| 지점 | 장애 | 기대 사항 |
|---|---|---|
| Gateway | Command 중복/순서 역전 | receipt/idempotency 유지 |
| Queue | full | 명확한 거부/대역폭 보호, RAM 폭증 없음 |
| Worker | Tick 도중 panic | Region 격리 + safe recovery |
| DB | BEGIN 이전 실패 | durable 상태 불변 |
| DB | UPDATE 후 COMMIT 전 kill | rollback |
| DB | COMMIT 직후 ACK 유실 | receipt 조회로 1회 확정 |
| Lease | old epoch로 쓰기 | DB fencing 거부 |
| Outbox | 중복 발행 | 소비자 멱등성 |
| Snapshot | checksum 손상 | 이전 snapshot fallback |
| LLM | 30s 지연/오답 | Tick 비차단, 불법 행동 거부 |
| Client | reconnect 중 delta gap | full snapshot sync |

## 5. 성능 목표(가정, 미측정)

| Metric | 임시 목표 | 측정 조건 |
|---|---|---|
| Tick frequency | 10Hz | single Active Region |
| `tick_duration_seconds` p95 | ≤30ms | 1k active NPC 중 실제 행동 10% |
| `tick_duration_seconds` p99 | ≤50ms | 동일 조건 |
| overrun (>100ms) | 1분 0.1% 미만 | 30분 부하 |
| DB durable command result | p95 ≤150ms | 동일 네트워크, 기본 거래 |
| WS snapshot resync | p95 ≤2s | 10~100 sessions |
| Region recovery | ≤30s 가정 | 1k NPC / 10MB snapshot |
| queue memory | bounded | backpressure 테스트 |

해당 숫자는 **승인/실측 전에 재조정 가능한 목표**이며, CPU 세대·NPC 수·전투율·규칙 수에 크게 좌우된다. 비교 시 서버 하드웨어/릴리스 빌드/seed/active 비율/OS를 기록.

## 6. Prometheus 메트릭 이름

| Metric | Type | Labels |
|---|---|---|
| `wse_tick_duration_seconds` | Histogram | region, mode |
| `wse_tick_overruns_total` | Counter | region |
| `wse_region_entities` | Gauge | region, entity_kind |
| `wse_npc_decisions_total` | Counter | region, goal_kind |
| `wse_goap_expansions_total` | Counter | region |
| `wse_command_queue_depth` | Gauge | region, queue_type |
| `wse_command_rejections_total` | Counter | region, reason |
| `wse_persistence_latency_seconds` | Histogram | operation |
| `wse_snapshot_age_seconds` | Gauge | region |
| `wse_region_epoch_changes_total` | Counter | region |
| `wse_replication_bytes_total` | Counter | region, message_type |
| `wse_client_resync_total` | Counter | reason |

개별 `entity_id/command_id`를 Prometheus label로 사용 금지(고카디널리티). 이들은 tracing/log에서 추적.

## 7. 구조화 로그/추적

JSON log의 공통 key: `timestamp`, `level`, `service`, `world_id`, `region_id`, `tick`, `epoch`, `command_id`, `event_id`, `actor_id`, `trace_id`, `error_code`, `duration_ms`. 사용자 비밀번호/토큰/민감 대화는 로깅 금지.

추천 Trace 흐름: `WS receive → admission → Region assigned tick → rule evaluation → durable intent → SQL commit → ACK → client final response`.

## 8. QA 통과 기준

- 기능 테스트 P0 항목 모두 PASS.
- 재생 가능한 사건 재현 데이터(`seed`, `inputs`, `rules`, `first_bad_tick`)를 버그에 첨부.
- durable 중복 거래·유실 및 두 Worker 동시 Commit 사례 **0건**.
- 성능 목표 미달이면 측정 결과와 병목 분석으로 승인 절차를 거친 뒤에만 예외 수용.
