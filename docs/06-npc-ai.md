# 06. NPC Decision Engine — Utility AI + GOAP (LLD)

## 1. 설계 목표

NPC는 플레이어가 없어도 목적·욕구에 따라 행동한다. 단, 모든 NPC를 매 Tick 재계획하지 않고 **Perception → Urgent Reaction → Utility Goal Selection → GOAP Planning → Action Execution** 단계를 분리한다. NPC AI는 영속 상태를 직접 변경하지 않으며 `NPC_ACTION` Command를 다음 Tick에 제출한다.

## 2. NPC 데이터 모델

| 구성요소 | 필드 | 저장/갱신 |
|---|---|---|
| Identity | stable_id, profession, home_region, faction_id | DB 영속 |
| Needs | hunger, fatigue, safety, wealth_pressure (0..1000) | ECS/체크포인트 |
| Personality | courage, greed, loyalty, curiosity (-1000..1000) | DB 영속 |
| Relationship | target_id, trust, affinity, fear, debt | DB 영속(중요 변화) |
| Memory | kind, target, severity, happened_tick, ttl, immutable refs | DB 중요 기억, 단기 ECS |
| Goals | survive, eat, earn, defend, revenge | AI state/체크포인트 |
| Plan | GOAP action list, current_index, precondition_hash | ECS/체크포인트 |
| AI Schedule | next_decision_tick, last_replan_tick | ECS/체크포인트 |

## 3. Perception — NPC가 아는 사실만 사용

- 주변 반경 `vision_range_tiles` 안의 Entity는 spatial hash/grid로 조회.
- `VisibleEntity`, `Threat`, `AvailableFood`, `NearbyWork`, `FactionAlert`, `KnownRoute` 등의 **관측 사실(Facts)** 생성.
- 다른 지역 내부 상태를 텔레파시처럼 즉시 읽지 않는다. 소문/메신저/세계 공지 등 전달된 사실만 반영.
- 지형/안개/시야 차단은 P1; MVP는 반경 + 장애물 단순 판정.

## 4. 긴급 반응과 일반 의사결정

**긴급 반응:** 피해/공격/기절/화재. 매 Tick 또는 이벤트 시 urgent queue에 삽입. `DODGE`, `FLEE`, `CALL_GUARD`, `COUNTERATTACK` 후보 평가. 1회 처리 후 cooldown 적용.

**일반 판단:** 초기 1초마다, NPC UUID의 안정된 해시로 Tick 분산. 현재 실행 중 행동이 유효하고 긴급 상황이 아니면 매번 목표를 바꾸지 않는다(히스테리시스).

## 5. Utility AI 점수 계산

각 목표 `g`는 입력 facts에 대해 0~1000 범위의 utility를 만든다.

\[
 U(g)=\operatorname{clamp}_{[0,1000]}(\sum_i w_{g,i}\,c_i(facts, needs) + p_g(personality) + q_g(context) - k_g(cooldown))
\]

- `c_i` = 0~1000 정수 정규화; `w` 고정소수점(예: 천분율), 내적에서 64bit intermediate 사용.
- 모든 weight와 curve는 버전 관리하는 데이터 파일에 저장.
- 최종 점수 차이가 `switch_margin=100` 이하이면 현재 goal 유지(목표 왕복 방지).
- 동점이면 `(goal_priority, stable_goal_id)`로 순서 결정.

**예: NPC 상태** `hunger=850`, `fatigue=400`, `threat=700`, `food_available=true`.

| 후보 | 정규화 입력/상황 | Utility 예시 | 결과 |
|---|---|---:|---|
| EAT | 배고픔 높고 음식 접근 가능 | 830 | 후보 |
| REST | 피로 중간, 주거 접근 가능 | 410 | 후보 |
| FLEE | 위협 높고 용기 낮음 | 930 | **선택** |
| WORK | 일터 있지만 위험 | 180 | 후보 |

수치는 예시이며 게임 밸런스 데이터로 외부화. `FLEE`를 선택하면 GOAP에 `Safe=true` 목표를 전달.

## 6. GOAP 플래너

### World Facts 예시

```rust
struct Facts {
    has_food: bool,
    near_home: bool,
    is_safe: bool,
    has_gold: bool,
    shop_open: bool,
}

struct GoapAction {
    id: &'static str,
    preconditions: Vec<FactPredicate>,
    effects: Vec<FactEffect>,
    cost_milli: u32,
    max_duration_ticks: u32,
}
```

### 기본 Action 정의

| Action | Precondition | Effect | Cost 예 |
|---|---|---|---:|
| MOVE_TO_MARKET | route_available, not_threatened | at_market=true | path_length×10 |
| BUY_FOOD | at_market, has_gold, stock>0 | has_food=true | price + queue |
| EAT_FOOD | has_food, not_in_combat | hunger_reduced=true | 200 |
| FLEE_TO_GUARD | threat_present, route_available | is_safe=true | route 위험도 |
| REST_AT_HOME | near_home, safe | fatigue_reduced=true | 500 |
| WORK_FIELD | season_ok, safe, tools_ok | earned_food=true | 700 |

### 탐색 알고리즘

1. Utility로 목표 상태(Goal Facts) 선택.
2. 현재 NPC Facts와 Action Catalog를 로드.
3. A* 또는 Dijkstra 기반으로 precondition/effect 가능한 노드 확장.
4. `max_expansions=128`, `max_plan_length=12`, `max_plan_ms` 제한(벽시계는 예산/운영에만 사용, 결과 순서 결정에는 사용 금지).
5. 행동 cost는 `duration + risk + resource + social_cost`의 정수 가중 합.
6. 동점은 stable action ID 정렬. 탐색 실패 시 `fallback = REST/WAIT/FLEE`.
7. 실행은 다음 Tick의 명령으로 제출하고 실제 비용·재고를 서버에서 다시 검증.

### 재계획 조건

- 목표 달성, 전투/위협 변경, 행동 실패, 필요한 재료 소진, 경로 차단, 계획 유효기간 만료.
- 연속 실패 3회: exponential cooldown, plan diagnostics 기록, 안전한 Idle로 전환.

## 7. GOAP 실행 상태 머신

`IDLE → EVALUATING → PLANNING → EXECUTING → (SUCCESS | FAILED | INTERRUPTED) → EVALUATING`.

각 Action은 `action_id`, `entity_id`, `plan_version`, `started_tick`, `timeout_tick`, `target_id`, `status`를 기록. Timeout에 도달했거나 stale `plan_version`이면 안전히 취소. 반복되는 액션 실패 시 `NPC_STUCK` 진단 이벤트를 기록한다.

## 8. NPC 기억/관계 설계

- 단기 기억: 시야에서 본 대상, 최근 대화, 현재 위협. 메모리 TTL(게임 시간) 만료.
- 장기 기억: 구조/배신/살인/보상 등 중요한 사건, DB에 보존.
- 관계: `(subject_id, target_id)` 쌍의 trust/affinity/fear 값을 정수로 저장.
- 사건 이후 후속 업데이트는 `RelationshipChangeCommand`로 제출하고 DB Commit 후 ECS projection 갱신.
- 기억의 주요 항목에는 `source_event_id` 고유성을 부여해 같은 사건 중복 학습 방지.

## 9. LLM 경계

```
Dialogue request (async) -> NPC facts + policy-approved memory
  -> LLM proposal (text, optional suggested_action)
  -> schema parse / content policy / rule validation
  -> dialogue broadcast or Command queued in future Tick
```

- LLM은 NPC의 실제 재산·아이템·위치 DB를 바꾸지 않는다.
- 제안 행동은 권한·가능성·자원·관계·거리 재검증 후 적용.
- timeout 시 템플릿 대사/Utility 결정으로 fallback; response는 idempotency key와 요청된 world revision 확인.
- 개인정보/대화 원문을 장기 기억에 그대로 저장하지 않는다; 목적·보존정책에 맞춘 요약만 저장.

## 10. 의사결정 성능

- `1,000 NPC / 1초 판단` 목표 시 10Hz Tick에는 평균 약 100 NPC 판단 분산.
- Perception은 spatial index를 사용하고 GOAP 전체 재계획은 목표 변동 때만 실행.
- `npc_decisions_total`, `goap_expansions_total`, `npc_plan_failures_total`, `npc_stuck_total` 계측.

## 11. 테스트/인수 기준

- TEST-AI-001: 배고픔이 높지만 급박한 위협이 있으면 FLEE가 EAT보다 우선.
- TEST-AI-002: 음식이 없어도 금화와 상점이 있으면 BUY → EAT 계획 생성.
- TEST-AI-003: 무효 경로 시 재계획, 연속 실패시 Idle 및 진단.
- TEST-AI-004: 같은 입력·seed·facts는 동일 goal/plan 생성.
- TEST-AI-005: LLM이 존재하지 않는 아이템 지급을 제안해도 실제 지급 0.
- TEST-AI-006: 중요 기억 source_event_id 중복 반영 0.
