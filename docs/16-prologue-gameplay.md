# 16. 플레이어 경험 · 프롤로그 · 공유 세계 진입 설계

## 1. 목적

게임이 시뮬레이션 중심이어도 첫 15분에 플레이어가 할 일을 알 수 있어야 한다. 개인 프롤로그는 핵심 기능을 순서 있게 보여주지만, 후반의 공유 세계는 자율 사건 중심으로 운영한다.

## 2. 15분 프롤로그 흐름(초기 가정)

| 단계 | 플레이어 행동 | 학습/검증 포인트 | 실패·대체 경로 |
|---|---|---|---|
| P0 | 개척촌 입장·NPC 대화 | 이동/상호작용 | 대화 실패시 다시 시도/강조 표시 |
| P1 | 자원 획득·NPC 도움 | 소유권·인벤토리 | 수용량 부족시 보관함/회수 |
| P2 | 국경 전초기지 이동 | 자동 경로 탐색 | 막히면 재탐색/대체 길 |
| P3 | 카엘 구조·고블린 전투 | 전투/피해/관계 | 패배/이탈해도 복구 가능한 상태 |
| P4 | 보급품 챙겨 이주 | 거래·자원·이벤트 | 선행 이벤트 자동 복구 점검 |
| P5 | 공유 정착지 도착 | 개인→공유 world transfer | timeout 시 transfer receipt 재조회 |

## 3. 프롤로그 상태 머신

`NOT_STARTED → IN_PROGRESS → READY_TO_TRANSFER → TRANSFERRING → COMPLETED`. 전이와 `completed_at`, `transfer_id`, `reward_receipts`는 DB durable 기록.

- 모든 이벤트에 `prerequisites`, `completion`, `timeout`, `fallback`, `diagnostics` 필드 부여.
- 사용자 행동의 선택지를 무리하게 한 경로로 고정하지 않고, 필수 학습 기능만 게이트로 둔다.
- 카엘 구조 이벤트 이후 다음 이벤트가 멈추는 버그 방지를 위해 `READY_TO_TRANSFER` 진입 조건을 **단일 선행 이벤트 완료**에만 의존하지 않도록 복구 경로 정의.
- 서버에 `COMPLETED`가 확정되면 클라이언트는 **개인 프롤로그 시작 버튼을 숨긴다**. UI 로컬 상태만 믿지 말고 서버 상태 기반 표시.
- `transfer_id` 재시도는 idempotent. NPC/아이템/캐릭터의 공유 세계 중복 생성 금지.

## 4. UI/게임 상태 계약

| UI | 서버 소유 상태 | 주의 |
|---|---|---|
| 자동 이동 버튼 | Navigation state | 수신/경로탐색/이동중/완료/실패 표시 |
| 대화 선택지 | Interaction availability | 거리/권한/대화 세션 재검증 |
| 보급품 취득 | Durable Inventory receipt | 성공 ACK 전 획득 표시 확정 금지 |
| 프롤로그 시작 버튼 | Prologue state | COMPLETED이면 숨김 |
| 월드 뉴스 | Confirmed domain events | 미확정 이벤트를 최종 사실로 노출 금지 |

## 5. 흥미를 유지하는 동적 상황

프롤로그 이후 공유 정착지에는 `Food shortage → Caravan ambush → Guard call → Player/NPC intervention` 루프를 제공한다. 플레이어가 방관해도 NPC가 결과를 만들고, 상점/치안/관계에 변화가 나타나도록 한다. 월드 변화는 지역 뉴스·NPC 대사·가격 지표로 관찰 가능해야 한다.

## 6. 테스트 및 DoD

- TEST-PRO-001: 신규 계정 프롤로그 시작→완료→공유 세계 전환.
- TEST-PRO-002: 모든 이벤트 단계에서 접속 끊김/재접속 후 계속 진행.
- TEST-PRO-003: READY_TO_TRANSFER 후 같은 transfer 재시도 100회, 캐릭터/아이템 중복 0.
- TEST-PRO-004: 완료 후 새 접속의 프롤로그 버튼 숨김.
- TEST-PRO-005: 자동 이동 실패 시 5초 이내 상태/사유 표시(초기 UX 목표).
- TEST-PRO-006: 경로/전투 실패·NPC 사망 때문에 필수 튜토리얼이 영구적으로 막히지 않음.
