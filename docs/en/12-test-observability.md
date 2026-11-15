# 12. Test Strategy, Benchmarks, and Observability

[한국어](../12-test-observability.md) | **English**

## 1. Quality gates

Every TASK must pass applicable stages of `unit → integration → deterministic → recovery → load`. Mark unexecuted tests `NOT RUN`; never present expected performance as measurements.

## 2. Test layers

| Layer | Target | Example tools |
|---|---|---|
| Unit | Damage, AI scoring, pathfinding, utility | Rust `cargo test` |
| Property | Resource conservation, ordering/boundaries, invariants | proptest |
| Deterministic | Snapshot hashes for identical inputs/seed/version | Headless runner |
| Integration | Command→Region→Writer→DB→ACK | Docker Testcontainers or temporary PG |
| Fault | Crash/ACK loss/DB disconnect/lease fencing | Failpoints + process termination |
| Protocol | Schema/codegen/WS state machine | Client/server fixtures |
| Soak | 24–72h simulation | Benchmark runner |
| Load | 1k/5k/10k NPCs; 10/100/500 sessions | k6/custom WS load generator |

## 3. Determinism verification

### Golden test fixture

```yaml
fixture_id: village_food_shortage_v1
engine_version: 0.1.0
rules_version: 1
world_seed: 12345
region_id: 8231eeec-2bf0-4a87-8671-d61016828889
initial_snapshot: fixtures/village_initial.bin
inputs: fixtures/village_inputs.jsonl  # includes assigned_tick + ingress_seq
run_ticks: 10000
expected_state_hash: "<record-after-first-validated-run>"
expected_event_hash: "<record-after-first-validated-run>"
```

Never populate `expected_*` without actual execution/review. Save an approved build's initial results as the golden baseline and record the reason/review for changes.

### Procedure

1. Load the latest snapshot, RNG, and rules version.
2. Inject input logs in `(assigned_tick, ingress_seq)` order.
3. Run 10,000 Ticks.
4. Sort by stable UUID, normalize/serialize state, and calculate SHA-256.
5. Normalize domain events by `(region_event_seq,event_type,payload)` and hash.
6. Compare against baseline; report the first divergence Tick and entity/event IDs.

## 4. Fault-injection matrix

| Location | Fault | Expected behavior |
|---|---|---|
| Gateway | Duplicate/reordered Commands | Preserve receipts/idempotency |
| Queue | Full | Explicit rejection/bandwidth protection; no RAM explosion |
| Worker | Mid-Tick panic | Region isolation + safe recovery |
| DB | Failure before BEGIN | Durable state unchanged |
| DB | Kill after UPDATE, before COMMIT | Rollback |
| DB | ACK loss immediately after COMMIT | Receipt resolves one finalization |
| Lease | Write with old epoch | DB fencing rejects |
| Outbox | Duplicate publication | Consumer idempotency |
| Snapshot | Corrupted checksum | Fall back to previous snapshot |
| LLM | 30s delay/wrong answer | Nonblocking Tick; reject illegal actions |
| Client | Delta gap during reconnect | Full snapshot sync |

## 5. Performance targets (assumptions, unmeasured)

| Metric | Provisional target | Conditions |
|---|---|---|
| Tick frequency | 10Hz | One Active Region |
| `tick_duration_seconds` p95 | ≤30ms | 1k active NPCs, 10% actually acting |
| `tick_duration_seconds` p99 | ≤50ms | Same conditions |
| Overrun (>100ms) | Under 0.1% per minute | 30-minute load run |
| DB durable command result | p95 ≤150ms | Same network, basic transactions |
| WS snapshot resync | p95 ≤2s | 10–100 sessions |
| Region recovery | Assumed ≤30s | 1k NPCs / 10MB snapshot |
| Queue memory | Bounded | Backpressure test |

These **targets may change before approval/measurement** and depend heavily on CPU generation, NPC count, combat ratio, and rule count. Record hardware, release build, seed, active ratio, and OS for comparisons.

## 6. Prometheus metrics

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

Do not use individual `entity_id/command_id` as Prometheus labels (high cardinality); track them in tracing/logs.

## 7. Structured logging and tracing

Common JSON keys: `timestamp`, `level`, `service`, `world_id`, `region_id`, `tick`, `epoch`, `command_id`, `event_id`, `actor_id`, `trace_id`, `error_code`, `duration_ms`. Never log passwords, tokens, or sensitive dialogue.

Suggested trace: `WS receive → admission → Region assigned tick → rule evaluation → durable intent → SQL commit → ACK → client final response`.

## 8. QA acceptance

- All P0 functional tests PASS.
- Attach reproducible event data (`seed`, `inputs`, `rules`, `first_bad_tick`) to bugs.
- **Zero** duplicate/lost durable transactions or concurrent commits by two Workers.
- Accept performance exceptions only after approval based on measured results and bottleneck analysis.
