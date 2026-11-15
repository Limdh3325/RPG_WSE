# 01. Software Requirements Specification (SRS)

[한국어](../01-requirements.md) | **English**

## 1. Functional requirements

| ID | Priority | Requirement | Acceptance criterion |
|---|---|---|---|
| REQ-F-001 | P0 | Headless execution without clients | Support `run --seed X --ticks 10000` |
| REQ-F-002 | P0 | Region ECS/stable IDs | Restore the same Game Entity UUID after restart |
| REQ-F-003 | P0 | Fixed 10Hz ticks and separate time scale | Control Tick/WorldTime independently |
| REQ-F-004 | P0 | Command validation/idempotency | Repeated purchase Command commits once |
| REQ-F-005 | P0 | Region Single Writer | Two Workers cannot commit concurrently in the same Epoch |
| REQ-F-006 | P0 | Autonomous NPC behavior over game time | NPC states/tasks change without input |
| REQ-F-007 | P0 | Movement/collision/pathfinding | Record failure/repath reasons for unreachable destinations |
| REQ-F-008 | P0 | Basic combat/damage/death | Validate HP, collision, range, and cooldown |
| REQ-F-009 | P0 | Event rules/follow-up events | Trace causality through event IDs |
| REQ-F-010 | P0 | Persistent state/restart recovery | Committed assets survive restart |
| REQ-F-011 | P0 | WS client state synchronization | Two clients agree on the latest Snapshot |
| REQ-F-012 | P1 | Active/Background Regions | Aggregate NPC simulation in idle areas |
| REQ-F-013 | P1 | Food production/consumption and market pricing | Stock shortages affect prices/events |
| REQ-F-014 | P1 | NPC relationships/memory/goals | Structured memories influence decisions |
| REQ-F-015 | P1 | Death/economic effects in the world | Consistently update world history/reputation/stock |
| REQ-F-016 | P1 | Private prologue → shared world | Transfer once without duplicate characters/items |
| REQ-F-017 | P1 | Operations tools | Region pause/resume, health, diagnostics |
| REQ-F-018 | P2 | LLM dialogue/action proposals | No world mutation before rule validation |
| REQ-F-019 | P2 | Multi-server Region transfer | Safe authority handoff through Epochs |

## 2. Nonfunctional requirements

| ID | Requirement | Initial target / verification |
|---|---|---|
| REQ-N-001 | Reproducibility | Same engine/rules version, seed, and input log produce identical event/state hashes |
| REQ-N-002 | Tick performance | 10Hz; Active Region p95 ≤30ms, p99 ≤50ms (**targets**) |
| REQ-N-003 | Load | Measure 1,000 NPCs/Region; 10,000 NPCs are a distribution/aggregation experiment |
| REQ-N-004 | Fault tolerance | No duplicate transaction when retrying after Commit with lost ACK |
| REQ-N-005 | Bounded memory | Bounded channels and explicit backlog response/drop policies |
| REQ-N-006 | Security | Server authority, Actor ownership, WSS/TLS, operations API RBAC |
| REQ-N-007 | Observability | Tick, queue, error, DB, recovery, event, and Region-state metrics |
| REQ-N-008 | Evolvability | JSON/Protobuf/DB schema_version and migration policy |
| REQ-N-009 | Data retention | No loss of durable events; positions recover to the latest successful checkpoint |

## 3. Assumptions and constraints

- Start with one Region on one server. Phase 1 does not require distributed consensus/Kafka.
- The **server validates all** game-state changes; clients render.
- Rust server; Vue 3 + Phaser UI; LLM through a separate asynchronous adapter.
- **PostgreSQL 16+ assumed**. 3D/elevation/continuous physics are outside the MVP.
- In-memory ECS owns Region components; the asset DB is the final authority for committed property.

## 4. Shared Definition of Done

1. Reflect RFC/ADR changes in documentation.
2. Version code, migrations, and message schemas.
3. Automate success, failure, duplicate, and restart-path tests.
4. Pass `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test`.
5. Make key metrics available in Grafana/Prometheus.
6. Log applicable IDs: `world_id`, `region_id`, `tick`, `command_id`, `trace_id`.
7. Record unmet performance targets with actual measurements and follow-up tasks; do not claim completion falsely.
