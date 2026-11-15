# 18. End-to-End Implementation Example — Purchases and Food Shortages

[한국어](../18-end-to-end-example.md) | **English**

This is an **integration-scenario contract** for validating how Gateway/Region/DB/Event/Client implementations from different contributors form one flow.

## A. Purchase: one item in stock, two concurrent players

### Initial state

- `market_stock(settlement_id,sku='FOOD',quantity=1,price=10,version=4)`.
- Players A and B each have 20 gold.
- Both clients request `BUY_ITEM quantity=1` using different `command_id` values.

### Server processing

1. Gateway verifies session→Actor→Region and validates input.
2. Region Tick assigns `assigned_tick`/`ingress_seq` and validates domain rules.
3. First command **reserves** one FOOD and 10 gold. Reject the second request considering the reservation, or reevaluate while pending.
4. Send the first DurableIntent to Writer; Tick continues processing other entities.
5. Writer verifies Region lease epoch and locks wallet/market/inventory rows in fixed order.
6. Revalidate stock/gold, change DB values, and write receipt/event/outbox in the same transaction.
7. ACK after COMMIT; Region releases reservations and updates committed ECS projection.
8. Gateway sends A `COMMAND_STATUS.SUCCEEDED` and stock/gold delta; B receives `OUT_OF_STOCK`.

### Core SQL outline (inside a transaction)

```sql
-- 0) Assume command_receipts unique key/hash and Region lease epoch validated
-- 1) Always lock in order: market_stock -> wallets -> inventory_slots
SELECT quantity, price, version
  FROM market_stock
 WHERE settlement_id = $1 AND sku = $2
 FOR UPDATE;

SELECT gold, version FROM wallets WHERE entity_id = $3 FOR UPDATE;

-- Apply only with sufficient gold/stock and valid versions
UPDATE market_stock SET quantity = quantity - 1, version = version + 1
 WHERE settlement_id = $1 AND sku = $2 AND quantity >= 1;
UPDATE wallets SET gold = gold - $4, version = version + 1
 WHERE entity_id = $3 AND gold >= $4;

-- UPSERT existing inventory_slots using the same row-lock/serial order.
-- Increase quantity once after durable command-ID idempotency checks.
-- Write domain_events, outbox, and receipt within the same BEGIN...COMMIT.
```

**Important:** this is deliberately an outline, not a complete executable transaction. Test UPDATE `RETURNING` row counts, command_id uniqueness, source/product ownership, server-calculated price, versions, epoch fencing, and concurrent insertion of missing slots.

### Failure reproduction

- If the Worker dies immediately after COMMIT, A's purchase remains applied once.
- If only the ACK is lost, query the receipt; do not purchase again during retry.
- B cannot buy until a later Event restores stock to one.

## B. Events from NPC production/consumption

### Initial state

`population=20`, `food_stock=100`, `target_stock=200`, `security=400`, `shortage_active=false`.

### Time progression

1. World/Economy applies production/consumption in game-time steps.
2. When `food_stock < shortage_threshold` changes **false → true**, an Event Rule schedules `RAISE_FOOD_SHORTAGE`.
3. Next Tick's Handler revalidates the condition and `shortage_active=false`.
4. For durable changes, finalize `FOOD_SHORTAGE_STARTED` only after DB ACK.
5. Outbox/Region rules schedule merchant escort/caravan requests under one causal correlation ID.
6. Merchant Utility AI evaluates trade; GOAP plans `BUY_SUPPLIES → TRAVEL → DELIVER`.
7. Bandits perceive low security/high trade volume and propose an ambush.
8. Players escort, attack, or observe; actual food_stock/security/reputation reflect the outcome.

### Event-flow tests

| TEST ID | State/action | Expected |
|---|---|---|
| TEST-E2E-001 | food_stock below threshold | One shortage event, visible news |
| TEST-E2E-002 | Remain below threshold for 1,000 Ticks | No repeated shortage event |
| TEST-E2E-003 | Successful caravan raid | Actual supply shortage/security changes |
| TEST-E2E-004 | Player escorts caravan | Survival, increased market stock, changed relationships |
| TEST-E2E-005 | Block NPC GOAP route | Repath or explicit failure; no stalled event |
| TEST-E2E-006 | Stop DB Writer | Durable goods/rewards never appear finalized |

## C. Acceptance

Reproduce 10,000 Ticks from the start with the same Seed/Rules/input log. Trace economy/AI/combat/rewards/news via E2E `correlation_id`. DB Commit, ACK loss, and multi-client reconnects must never duplicate transactions.
