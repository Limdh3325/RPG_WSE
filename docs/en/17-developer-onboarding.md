# 17. Developer Onboarding and Getting Started

[한국어](../17-developer-onboarding.md) | **English**

## 1. First steps

### Step 1. Understand intent and scope

Read `README → 00-vision-scope → 01-requirements → 02-hld-architecture → 15-adrs`. Then read relevant modules; Tick developers should read 03, 04, 05, and 12.

### Step 2. Prepare the minimum environment

- Rust stable + Cargo, pinned by the team in `rust-toolchain.toml`.
- PostgreSQL 16+ test instance for persistence work.
- `cargo fmt`, `cargo clippy`, `cargo test`.
- Docker/container runtime for integration testing.
- Git, a DB migration tool such as `sqlx-cli`, and Grafana/Prometheus for performance work.

```bash
# Baseline commands after the team prepares the repository
rustc --version
cargo --version
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Historical design-stage note:** the original package was a design-document set rather than a Cargo implementation; TASK-001 was to create Cargo.toml/Workspace/code and pin Rust/SQLx versions in the first implementation PR. For the current checkout, use the [root README](../../README.en.md), which documents the existing Workspace and execution limits.

### Step 3. Choose a first task

- Engine: `TASK-001 → TASK-002 → TASK-003`.
- Domain: `TASK-004 → TASK-005 → TASK-010/011`.
- DB: `TASK-007 → TASK-008`.
- Server/client: `TASK-006 → TASK-009 → TASK-012`.
- QA/operations: `TASK-013 → TASK-014`.

**Parallel-work prerequisite:** review API/event/DB contracts before independently developing dependent modules. Request interface-owner review for shared-contract changes.

## 2. Questions to answer before coding

1. Is the **source of truth** ECS or PostgreSQL?
2. Who is the **sole Writer**?
3. What happens if the same Command repeats 100 times?
4. How does recovery work if the server dies just after DB Commit?
5. Which Tick applies the work, and in what Replay order?
6. Which events/metrics/tests reveal success and failure?

If any answer is unclear, improve the ADR/design before opening an implementation PR.

## 3. Directories and owners

| Directory | Responsibility | Reviewers |
|---|---|---|
| `crates/simulation-core` | Tick, Region, ownership | Core/Architect |
| `crates/ecs-components` | Component contracts, Stable ID | Core/Domain |
| `crates/simulation-systems` | Movement/Combat/NPC/Economy | Domain/Core |
| `crates/game-domain` | Command/Event/rules | Architect/Domain |
| `crates/persistence` | DB transaction/snapshot/recovery | DB/Core |
| `crates/networking` | Axum/session/AOI/delta | Backend/Security |
| `crates/shared-protocol` | External message versions | Backend/Frontend |
| `apps/simulation-runner` | Deterministic replay/load | QA/Core |

## 4. Example: adding an NPC action

1. Identify requirements (for example `REQ-F-006`) and TEST IDs.
2. Define `action_id`, preconditions, effects, cost/timeout in the catalog.
3. GOAP schedules execution Commands; it must not mutate state directly.
4. Next Tick's Handler revalidates distance/resources/state.
5. Durable outcomes go through DB Writer receipt/transaction/ACK.
6. Add rules if Domain Events affect NPC memories/quests.
7. Pass golden replay/failure/duplicate tests and inspect event traces.

## 5. Initial troubleshooting

| Symptom | First checks |
|---|---|
| Auto-move click does nothing | Client COMMAND→Gateway receipt→Region queue→Navigation state→Reject reason |
| NPC stops | next_decision_tick, Utility score, GOAP failure, budget exhaustion |
| Quest does not advance | Rule prerequisite, event causation, cooldown, tick, outbox/retry |
| Duplicate items | command_id, receipt hash, DB tx/unique, actor reservation |
| Reconnect mismatch | Snapshot baseline, delta gap, Region revision, reconnect path |
| Worker restart loop | Lease epoch, checksum, migration/upcaster, reconciliation |
| Performance degradation | Per-system Tick duration, path expansions, AI evaluations, channel depth |

## 6. Review/handoff format

Record this for each completed TASK:

```markdown
### Task completion: TASK-XXX
- Implemented code locations:
- Documentation/ADR changes:
- API/DB/event schema_version:
- Passing tests and commands:
- Measured performance and environment:
- Failure recovery/rollback procedure:
- Known limitations and follow-up TASKs:
```
