# 05. Command/Event Pipeline and Execution Rules (LLD)

[한국어](../05-command-event.md) | **English**

## 1. Invariants and responsibilities

- Command = intent; Event = an occurred fact; Delta = client-visible current-state change. Do not conflate them.
- `CommandEnvelope` includes `command_id(UUID)`, `actor_id(UUID)`, `region_id(UUID)`, `session_id(UUID)`, `client_seq`, `assigned_tick`, `ingress_seq`, `ownership_epoch`, `schema_version`, and `payload`.
- `ingress_seq` is a Region-local monotonically increasing number assigned by the Worker when admitting queued input. This is the authoritative replay order.
- Do not trust client `target_tick`; the Region assigns `assigned_tick` at admission.
- NPC/LLM/operations commands use the same Pipeline with different origins and permission scopes.

## 2. Command processing contract

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

`Received` means received by Gateway; `Accepted` means Region-validated/queued; `PendingCommit` means a durable intent is not yet DB-finalized; `Succeeded` means successful execution. Even simple movement is rejected if tile, speed, or state validation fails.

### Validation order

1. Check WSS authentication, session TTL, and Actor ownership.
2. Validate protocol_version, payload size, and rate limits.
3. Check region_id/epoch in the Router.
4. In the Region Tick, validate actor state, distance, resources, cooldowns, and world rules.
5. Apply transient effects or create durable reservations.
6. Record events/responses and return the result to the Session.

## 3. Deterministic ordering and deduplication

- Default order: `(assigned_tick, ingress_seq)`; monotonically increasing `ingress_seq` makes it unique.
- Replay input logs include **validation results and application order**.
- Durable commands store a DB receipt with `UNIQUE(command_id)`. Same ID with different payload returns `IDEMPOTENCY_CONFLICT`.
- Transient deduplication uses a session/sequence LRU window. Replayed movement after restart may be discarded after snapshot resync.
- Retain receipts for at least the operational policy period; retain purchase/ownership receipts indefinitely or under long-term audit policy.

## 4. Event structure

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

- Durable `region_event_seq` increases monotonically in DB Commit order.
- Transient events may use `tick_seq` and do not consume durable event sequence numbers.
- Use `causation_command_id` and `correlation_id` to trace event chains.

## 5. Event rule format

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

**Important:** do not execute expressions using arbitrary string `eval`. Use enums/ASTs or a validated DSL parser with allowed fields/operators. Event definitions propose rule Commands; they cannot directly run arbitrary SQL or mutate the world.

## 6. Event-chain limits

- Normally schedule follow-up commands for `tick+1` or later.
- Enforce `max_events_per_tick`, `max_causation_depth`, per-rule cooldowns, and `event_id` deduplication.
- On infinite chains/invariant violations, emit `RULE_CYCLE_DETECTED`, disable only the offending rule, and alert operations.
- Historical Event Replay must not reissue side effects such as LLM calls, email, or external Webhooks. Outbox publishers implement their own idempotency.

## 7. Transaction finalization and failure modes

`Validate → Reserve → DurableIntent → DB Transaction → ACK → Commit ECS projection → SUCCEEDED`.

- Reserved currency/items cannot be reused while Pending.
- DB Timeout does not imply failure. Look up the Receipt; keep unresolved outcomes `UNKNOWN/PENDING`. Never arbitrarily refund or re-execute.
- Receipt of a request alone does not justify a success response.
- Write event log, state tables, Outbox, and Receipt in **one DB transaction**.

## 8. Tests

- TEST-CMD-001: repeat one purchase Command 100 times; deduct currency once.
- TEST-CMD-002: same command_id, different payload → `IDEMPOTENCY_CONFLICT`.
- TEST-EVENT-001: one shortage-condition activation produces one FOOD_SHORTAGE.
- TEST-EVENT-002: cyclic rules stop at the depth limit with traceable causality.
- TEST-EVENT-003: resolve success/failure by receipt lookup after ACK loss.
- TEST-EVENT-004: zero external Webhook/LLM reissues during Replay.
