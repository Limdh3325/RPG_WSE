# 18. 끝까지 따라가는 구현 예제 — 구매와 식량 부족

**한국어** | [English](en/18-end-to-end-example.md)

이 문서는 서로 다른 개발자가 작성한 Gateway/Region/DB/Event/Client가 어떻게 한 흐름으로 연결되는지 검증하는 **통합 시나리오 계약**이다.

## A. 구매: 재고 1개, 동시 플레이어 2명

### 초기 상태

- `market_stock(settlement_id,sku='FOOD',quantity=1,price=10,version=4)`
- 플레이어 A 금화 20, 플레이어 B 금화 20.
- 두 Client가 각각 다른 `command_id`로 `BUY_ITEM quantity=1` 요청.

### 서버 처리

1. Gateway 세션→Actor→Region 확인, input 검증.
2. Region Tick은 `assigned_tick`, `ingress_seq`를 확정하고 도메인 검증.
3. 첫 명령은 FOOD 1개와 금화 10을 **예약**. 동일 재고에 대한 두 번째 요청은 예약을 고려해 거부하거나 pending 상태에서 재평가.
4. 첫 DurableIntent를 Writer로 전달. Tick은 다른 Entity 처리 계속.
5. Writer는 Region lease epoch를 검증한 뒤 wallet/market/inventory를 고정된 순서로 row lock.
6. 재고와 금화를 재검증하고 DB 값을 변경; receipt/event/outbox 같은 트랜잭션에 기록.
7. COMMIT 후 ACK; Region은 예약 해제하고 ECS committed projection 반영.
8. Gateway는 A에게 `COMMAND_STATUS.SUCCEEDED` 및 새 재고/금화 delta 전달. B에게는 `OUT_OF_STOCK`.

### 핵심 SQL 윤곽 (트랜잭션 내부 예시)

```sql
-- 0) command_receipts 고유 키/해시 및 Region lease epoch 검증 완료 가정
-- 1) 잠금 순서는 항상 market_stock -> wallets -> inventory_slots로 고정
SELECT quantity, price, version
  FROM market_stock
 WHERE settlement_id = $1 AND sku = $2
 FOR UPDATE;

SELECT gold, version FROM wallets WHERE entity_id = $3 FOR UPDATE;

-- 금화 충분/stock 충분/Version 합당한 경우에만 적용
UPDATE market_stock SET quantity = quantity - 1, version = version + 1
 WHERE settlement_id = $1 AND sku = $2 AND quantity >= 1;
UPDATE wallets SET gold = gold - $4, version = version + 1
 WHERE entity_id = $3 AND gold >= $4;

-- inventory_slots는 기존 행이면 UPSERT: 동일 row lock/serial order를
-- 맞추고, 영속 명령 ID 멱등 처리 후 1회만 quantity 추가한다.
-- domain_events, outbox, receipt는 동일 BEGIN...COMMIT 내에서 기록.
```

**주의:** SQL은 의도적인 윤곽이며 바로 실행 가능한 전체 트랜잭션이 아니다. 실제 구현에서는 각 UPDATE의 `RETURNING` 행 수, `command_id` uniqueness, 출처/상품 보유 여부, 서버가 계산한 가격, 버전, epoch fencing, 부재한 slot의 동시 insert 처리까지 테스트로 검증해야 한다.

### 장애 재현

- COMMIT 이후 Worker가 즉시 죽어도 A 구매 1회만 유지.
- ACK만 유실되면 재시도 중 구매하지 말고 receipt 상태 조회.
- B는 재고가 다시 1이 되는 후속 Event 전까지 구매할 수 없음.

## B. NPC 생산/소비에서 사건 생성

### 초기 상태

`population=20`, `food_stock=100`, `target_stock=200`, `security=400`, `shortage_active=false`.

### 시간 진행

1. World/Economy System이 게임 시간 단위로 생산/소비 반영.
2. `food_stock < shortage_threshold`가 **false → true**로 변경되면 Event Rule이 `RAISE_FOOD_SHORTAGE` Command 예약.
3. 다음 Tick Command Handler가 조건과 `shortage_active=false`를 재검증.
4. 변경이 durable이라면 DB ACK 이후 `FOOD_SHORTAGE_STARTED` Domain Event 확정.
5. Outbox/Region rule이 상인 호위/수송대 요청을 예약; 단일 원인 correlation ID 추적.
6. 상인 NPC는 Utility AI로 교역 목표를 평가, GOAP `BUY_SUPPLIES → TRAVEL → DELIVER` 계획 수립.
7. 도적 NPC는 낮은 치안·높은 교역량을 감지하여 습격을 제안.
8. 플레이어는 호위/공격/방관을 선택하고, 사건 결과가 실제 food_stock/security/reputation에 반영.

### 사건 전개 테스트

| TEST ID | 상태/행동 | 기대 |
|---|---|---|
| TEST-E2E-001 | `food_stock` 임계값 이하 | shortage 이벤트 1회, 뉴스 노출 |
| TEST-E2E-002 | 임계값 상태로 1,000 Tick 유지 | 반복 shortage 이벤트 없음 |
| TEST-E2E-003 | caravan 습격 성공 | 실제 보급 부족/치안 변화 |
| TEST-E2E-004 | player caravan 호위 | 생존·시장 재고 증가·관계 변화 |
| TEST-E2E-005 | NPC GOAP 경로 차단 | 재경로 또는 명시적 실패, 이벤트 정체 없음 |
| TEST-E2E-006 | DB Writer 중단 | durable 품목/보상이 최종 지급된 것으로 보이지 않음 |

## C. 인수 완료 기준

동일한 Seed/Rules/입력 로그에서 초기부터 10,000 Tick 결과를 재현한다. E2E Event의 `correlation_id`로 경제/AI/전투/보상/뉴스의 인과관계를 추적한다. DB Commit/ACK 유실/다중 클라이언트 재접속을 거쳐도 거래 이중 반영이 없어야 한다.
