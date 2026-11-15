# 05. Command / Event Pipeline · 실행 규칙 (LLD)

**한국어** | [English](en/05-command-event.md)

## 1. 불변식 및 책임

- Command = 의도, Event = 발생한 사실, Delta = 클라이언트 현재 상태 변화. 혼용 금지.
- `CommandEnvelope`는 `command_id(UUID)`, `actor_id(UUID)`, `region_id(UUID)`, `session_id(UUID)`, `client_seq`, `assigned_tick`, `ingress_seq`, `ownership_epoch`, `schema_version`, `payload`.
- `ingress_seq`는 Region Worker가 입력을 큐에서 받아 확정하는 지역 내부 단조 증가 번호. 재생에는 이 순서가 원본이다.
- Client `target_tick`은 신뢰하지 않는다. Region이 admission 시 `assigned_tick`을 지정한다.
- NPC/LLM/운영 명령도 동일 Pipeline 사용(출처와 권한 scope가 다름).

## 2. 명령 처리 계약

```rust
pub enum CommandPayload {
    MoveTo { x: i64, y: i64 },
    Attack { target_id: uuid::Uuid },
    Interact { target_id: uuid::Uuid },
    BuyItem { shop_id: uuid::Uuid, sku: String, quantity: i32 },
    NpcAction { action_id: String, target_id: Option<uuid::Uuid> },
}

pub enum CommandStatus {
    Received, Accepted, Rejected,
    PendingCommit, Succeeded, Failed,
}
```

`Received`=Gateway가 받은 상태, `Accepted`=Region 검증·큐잉, `PendingCommit`=durable intent DB 미확정, `Succeeded`=실행 성공. 단순 이동도 타일·속도·상태 검증 실패 시 거부된다.

### 검증 순서

1. WSS 인증/세션 TTL/Actor ownership 확인.
2. protocol_version/payload size/rate limit 검증.
3. Router에서 region_id/epoch 확인.
4. Region Tick에서 actor 상태, 거리, 자원, 쿨다운, 월드 규칙 확인.
5. transient effect 적용 또는 durable reservation 생성.
6. 이벤트/응답 기록, 결과를 Session에 반환.

## 3. 결정적 정렬/중복 처리

- 기본 정렬: `(assigned_tick, ingress_seq)`; `ingress_seq` 단조 증가이므로 유일함.
- 재현용 입력 로그는 **검증 결과와 적용 순서**까지 포함한다.
- durable 명령은 DB에 `UNIQUE(command_id)` receipt를 남긴다. 같은 ID + 다른 payload는 `IDEMPOTENCY_CONFLICT`.
- transient 명령 중복 제거는 session/sequence 기반 LRU 윈도우. 재시작 간 이동 입력 재전송은 snapshot 재동기화 후 폐기 가능.
- Command receipt는 최소 게임 운영 정책 기간 보존; 구매/소유권 관련 영속 receipt는 무제한 또는 감사 보존정책으로 장기 보존.

## 4. Event 구조

```json
{
  "event_id": "4c69723e-d891-4ea0-ac73-680c1d48f980",
  "event_type": "CARAVAN_DAMAGED",
  "schema_version": 1,
  "world_id": "<uuid>",
  "region_id": "<uuid>",
  "region_event_seq": 142,
  "tick": 992,
  "causation_command_id": "<uuid-or-null>",
  "correlation_id": "<uuid>",
  "source_entity_id": "<uuid>",
  "payload": {"caravan_id":"<uuid>", "hp_lost":12}
}
```

- `region_event_seq`는 durable 이벤트에 대해 DB Commit 순서 기준 단조 증가.
- transient 이벤트는 `tick_seq`로 구분 가능하며 durable event sequence를 소비하지 않는다.
- `causation_command_id`와 `correlation_id`로 연쇄 사건의 원인을 추적한다.

## 5. Event Rule 포맷

```yaml
rule_id: caravan_attack_causes_shortage_v1
priority: 100
scope: region
trigger: CARAVAN_DAMAGED
conditions:
  - expr: "settlement.food_stock < settlement.min_food_stock"
  - expr: "settlement.shortage_active == false"
commands:
  - type: RAISE_SETTLEMENT_CONDITION
    params: {condition: FOOD_SHORTAGE}
cooldown_game_seconds: 3600
max_occurrences_per_region_day: 1
```

**주의:** 표현식은 임의 문자열 `eval`로 실행하지 않는다. enum/AST 또는 검증된 DSL parser로 허용 필드와 연산만 지원. 이벤트 정의는 규칙 Command를 제안할 뿐, 직접 임의 SQL/월드 변경은 금지.

## 6. 이벤트 연쇄 제한

- 이벤트의 후속 명령은 원칙적으로 `tick+1` 이후 예약.
- 한 Tick `max_events_per_tick`, 인과관계 depth `max_causation_depth`, 규칙 단위 cooldown, `event_id` 중복 방지.
- 무한 연쇄 또는 불변식 위반 시 `RULE_CYCLE_DETECTED`를 발행하고 해당 rule만 비활성화/운영 알림.
- 과거 Event Replay는 side effect(LLM/이메일/외부 Webhook)를 재발행하지 않으며, outbox 발행자는 별도 멱등 처리.

## 7. 거래 확정 경로와 실패 모드

`Validate → Reserve → DurableIntent → DB Transaction → ACK → Commit ECS projection → SUCCEEDED`.

- Pending 동안 같은 금화/아이템을 재사용할 수 없음.
- DB Timeout은 실패와 동일하지 않음. Receipt 조회 후 결과가 불명확하면 `UNKNOWN/PENDING`으로 유지, 임의 환불/재실행 금지.
- 수신 성공만으로 성공 응답을 반환하지 않음.
- 이벤트 로그와 상태 테이블/Outbox/Receipt는 **한 DB 트랜잭션**에 기록.

## 8. 테스트

- TEST-CMD-001: 동일 구매 Command 100회, 재화 차감 1회.
- TEST-CMD-002: command_id 동일·payload 다름 → `IDEMPOTENCY_CONFLICT`.
- TEST-EVENT-001: 식량 부족 조건 1회 충족 시 단일 FOOD_SHORTAGE.
- TEST-EVENT-002: 순환 Rule이 depth 제한에서 중단되고 원인 추적 가능.
- TEST-EVENT-003: ACK 손실 후 receipt lookup으로 성공/실패 구분.
- TEST-EVENT-004: Replay 중 외부 Webhook/LLM 재발행 0건.
