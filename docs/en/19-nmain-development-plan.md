# Detailed Development Plan for the New nmain Engine

[한국어](../19-nmain-development-plan.md) | **English**

Written: 2026-11-09 (Asia/Seoul).
Status: implementation kickoff plan. See 20-development-status.md and completion reports in outbox for progress.

The implementation target is a repository checkout in a developer-provided environment. Set server addresses/install paths for that environment. The starting state and nmain branch name below are historical records from when this plan was written.

## 1. Goal and starting point

Build a new Rust + Bevy ECS + Tokio + Axum + PostgreSQL + Vue 3/Phaser game from the README and docs/00–18 baseline. API/DB/save compatibility with the previous Go/React game is outside the first release. Automatic migration of old features is not assumed.

Repository state recorded at planning time:

- Working branch: nmain, locally tracking origin/nmain; remote freshness was not verified by fetch.
- Starting commit: e75fa61 `feat:init WSE`.
- main is at 3a715bc and is not changed by this planning work.
- Root README, docs/00–18, and schemas/config.example.toml are the design baseline.
- Cargo.toml and apps do not yet exist at this recorded starting point.
- schemas/0001_initial.sql exists locally but is excluded by `*.sql`; TASK-000 must track it for fresh checkouts.
- Resume.md and Resume_KR.md are user-owned untracked files and excluded from development changes.

The first goal is a small complete game: a 64×64 frontier village with one player, 10 NPCs, two monster types, food shortage/caravan-raid intervention, and committed assets surviving restart. Verify agreement between two clients before scaling to 10 concurrent players/1,000 NPCs.

## 2. Scope and implementation principles

- Start with one process and one shared Region. Add private-to-shared transfers during the prologue phase.
- One Worker writes each Region ECS. Network/DB/LLM work stays outside Tick.
- Separate Command (intent), Domain Event (confirmed fact), and State Delta (presentation) types.
- Finalize gold/inventory/ownership/important progress only after DB Commit confirmation. Resolve lost ACKs by receipt lookup rather than treating them as failure.
- Define checkpoint-bounded loss allowances for transient state such as position.
- Initially fix ECS execution order on one thread. Do not conflate independent Regions with parallel execution inside an ECS.
- Complete the first game using template dialogue and rule-based NPCs. LLM is an optional later feature.
- Distributed sharding, multi-server transfer, instant cross-region trade, advanced PvP compensation, large worlds, and old dungeon/passive/pantheon migration are follow-up work.
- Add Prometheus metrics/structured logs early; finalize dashboards/load verification later.

## 3. Planned directories and responsibilities

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
apps/
  game-server/          # Process assembly, config, runtime, shutdown
  simulation-runner/    # Seed/input logs/replay/hashes/benchmarks
  web-client/           # Vue 3 + TypeScript + Phaser
crates/
  game-domain/          # IDs, Command/Event, transaction/error types
  ecs-components/       # ECS components and StableId mapping
  simulation-core/      # Tick, Region execution, RNG, deterministic scheduling
  simulation-systems/   # Movement, combat, NPCs, economy, event rules
  persistence/          # SQLx, receipt, lease, outbox, snapshot
  networking/           # Axum, authentication, WS, AOI, replication
  shared-protocol/      # Versioned wire DTOs and contract fixtures
schemas/                # SQL migrations and example config
fixtures/               # Worlds, input logs, protocol, golden baselines
deploy/                 # New development/test Compose and monitoring
docs/                   # Designs, ADRs, plan, runbooks
```

Domain rules must not depend on Axum/SQLx. Message boundaries prevent persistence/networking from directly mutating ECS. Validate Rust/TypeScript contracts through shared fixtures; decide code generation in TASK-004. This is the planned structure; creation begins in TASK-001.

## 4. Milestones, tickets, and acceptance

### M0 — Reproducible development environment (TASK-000~001)

**TASK-000: repository/design baseline — no dependencies**

1. Inspect tracked files/remaining directories; distinguish new build entry points from legacy references. Do not bulk-delete.
2. Add Rust-output ignores and a `schemas/*.sql` exception; track initial SQL. Ensure fresh checkouts include config examples/migrations.
3. Check README links/execution/document precedence. Edit split designs as sources; define integrated-document/manifest regeneration to prevent drift.
4. Finalize a development Compose with separate DB name/volume/ports. Do not connect existing services/databases.
5. Create REQ→TASK→TEST mapping and progress tracking.

Done: fresh checkouts include designs/SQL and clearly identify the new game's execution path.

**TASK-001: Rust Workspace/CI — depends on TASK-000**

1. Create planned apps/crates; pin Rust edition 2024, compiler, and dependencies after compatibility checks.
2. Configure Cargo.lock, configuration loader/environment overrides, tracing, and error types.
3. Implement server health/liveness and Runner CLI; distinguish readiness from actual preparedness.
4. Add fmt/clippy/test CI; PostgreSQL integration jobs follow in TASK-007.
5. Provide new-development PostgreSQL Compose and `.env.example`.

Done: empty server/Runner execute; builds/checks reproduce from a clean checkout.

### M1 — Deterministic headless world (TASK-002~004)

**TASK-002: Region/Entity model — depends on TASK-001**

- Implement World/Region/Stable UUID↔ECS Entity mapping, creation/deletion/query/lifecycle.
- Build minimal Position/Velocity/Health/Needs/Inventory projections and a 64×64 test world.
- Separate canonical state DTOs from ECS handles; preserve external IDs after restore.
- Verify mapping consistency and duplicate-ID rejection across 10,000-Entity create/delete/recreate cycles.

**TASK-003: Fixed Tick/RNG/reproducibility — depends on TASK-002**

- Separate 100ms logical Tick from game-time scale; fix phase order.
- Decide RNG algorithm/version/stream separation. Treat UUID generation/external results as reproducible inputs or deterministic generation rules.
- Implement stable ordering, integer/fixed-point math, canonical serialization, and state/event hashes.
- Add seed, ticks, input fixtures, reports, and first-divergent-Tick output to Runner.
- Verify matching state/event order over repeated 10,000 Ticks with identical seed/initial state/input/version. Headless execution must not wait on wall-clock time.

**TASK-004: Command/Event contracts — depends on TASK-003**

- Define command_id, actor/region, payload_hash, assigned_tick, ingress_seq, and validation errors.
- Unify received/rejected/executing/pending-persistence/succeeded/failed transitions and status lookup contracts.
- Define event_id, causation_id, correlation_id, schema/rules versions, region_event_seq.
- Separate transient deduplication guarantees from durable DB receipts; reject command_id reuse with a different payload.
- Bound event budget/causal depth and schedule follow-up Ticks to prevent infinite chains.
- Verify reordered/duplicate input, rule cycles, and unknown version/field handling.

**M1 gate:** pass 10,000-Tick replay without UI/DB. In-memory deduplication at this stage is not restart durability.

### M2 — Movement, combat, and Worker boundaries (TASK-005~006)

**TASK-005: minimal movement/combat — depends on TASK-004**

- Implement integer millitile movement, tile collision, A* budgets, and dynamic-obstacle repathing.
- Expose received→pathfinding→moving→completed/failed states and reasons.
- Implement two monster types, range/cooldown/damage/death rules.
- Emit durable intents for loot/death; do not claim production-grade final grants before TASK-007.
- Verify wall/speed/out-of-range rejection and no-path/dead-target/duplicate-reward handling.

**TASK-006: Region Worker/Router — depends on TASK-004; integration requires TASK-005**

- Bound channels between dedicated Region execution threads and Tokio IO.
- Separate Command/ACK/Durable/Transient queues with ACK priority and saturation policy.
- Implement lifecycle, single writer, epoch hooks, pause/resume, graceful shutdown drain.
- Add two-Region isolation tests without distributed servers.
- Verify explicit overload rejection, unrelated Entity ticks during persistence delay, and cross-Region failure isolation.

### M3 — Asset finalization and restart recovery (TASK-007~008)

**TASK-007: PostgreSQL transactions/leases/outbox — depends on TASK-004/006**

- Validate initial SQL in a disposable empty PostgreSQL 16+ DB; implement versioned migrations/startup policy.
- Commit wallet/stock/inventory plus receipt/event/outbox in one transaction.
- Implement asset reservations, fixed lock order, version revalidation, lease/epoch fencing, and post-ACK ECS finalization.
- Resolve ACK timeouts through receipts; prevent repeated execution and payload-conflicting ID reuse.
- Implement Outbox redelivery/consumer deduplication; stop durable admission and expose state during DB failures.
- Verify two buyers/one item, 100 identical purchases, pre-Commit interruption, lost post-Commit ACK, stale writes, duplicate Outbox publication.

**TASK-008: checkpoint/recovery — depends on TASK-007**

- Stop new durable admission → resolve pending → snapshot at Tick boundary.
- Save engine/schema/rules versions, checksum, committed-event watermark, RNG, scheduled actions.
- Acquire new epoch → restore valid snapshot → reconcile committed DB data → verify invariants → activate.
- Fall back after snapshot corruption; explicitly fail on incompatible versions.
- Verify interruption during snapshot, checksum damage, post-Commit restart, timer/RNG restoration, and receipt resolution.

**M3 gate:** zero committed-asset loss/duplication with real PostgreSQL and process termination tests. Serialization unit tests alone do not establish recovery completion.

### M4 — Two clients observe one world (TASK-009, TASK-012A)

**TASK-009: authentication/WebSocket/AOI — depends on TASK-006/007; restart tests need TASK-008**

- Implement account/character ownership, sessions, short-lived single-use tickets, authentication timeout.
- Validate protocol/schema versions, message size/rate, and malformed input.
- Implement AOI cell subscriptions, baseline snapshot, delta_seq, ACK, gap/resync, slow-client isolation.
- Requery latest state and pending command results after reconnect.
- Test actor forgery, ticket reuse, two-client agreement, loss/duplication/disconnects, and queue backlog.

**TASK-012A: first playable screen — depends on TASK-005/009**

- Vue 3/TypeScript/Phaser app with tiles, characters, camera, direct movement.
- HP/dialogue/inventory and command progress/failure UI; display assets only after finalization.
- Validate gameplay with placeholder art first; record asset provenance/usage terms.
- Run unit/protocol tests and E2E with two actual browsers.

**M4 gate:** actual browser login→connect→move→combat→reconnect. A successful frontend build does not replace gameplay verification.

### M5 — First game with NPC life and event chains (TASK-010~011)

**TASK-010: Utility AI/GOAP — depends on TASK-002/003/005**

- Implement Perception/Needs/relationships/memories, goal scoring, limited GOAP actions/planning budgets.
- Configure farmers producing, merchants trading, guards patrolling, bandits raiding/fleeing.
- Stagger decisions across Ticks and submit next-Tick Commands; fall back on path failure/missing targets.
- Start with template dialogue; connect structured important memories to TASK-007 storage.
- Verify 10 autonomous NPCs, danger avoidance, blocked-plan recovery, and shared validation of invalid actions.

**TASK-011: food/market/caravans/events — depends on TASK-007/010**

- Implement production/consumption/stock/pricing/security/reputation, ledgers, and nonnegative/no-duplicate invariants.
- Trigger shortage on false→true threshold transition; suppress duplicates during sustained shortage.
- Connect shortage→supply request→caravan travel→bandit raid→escort/attack/observe→outcome.
- Use restricted rule expressions/validators; no arbitrary code execution.
- Display confirmed events through news/prices/NPC dialogue; provide timeout/fallback/diagnostics at every stage.
- Verify doc 18's purchases and TEST-E2E-001~006. NPC actions must resolve events even when players observe without acting.

**M5 gate:** complete the README's first Vertical Slice, including committed economic effects, restart recovery, and two-client consistency.

### M6 — Private prologue and shared-world entry (TASK-012B)

Dependencies: TASK-008/009/010/011/012A. Split original TASK-012 into screen construction and transfer so transfer completion does not precede recovery.

- Create private Regions; implement P0 entry/dialogue→P1 resources→P2 travel→P3 rescue/combat→P4 supplies→P5 shared settlement.
- Persist NOT_STARTED→IN_PROGRESS→READY_TO_TRANSFER→TRANSFERRING→COMPLETED.
- Make character/NPC/item transfers idempotent with transfer_id/reward receipts; support one process/one DB only.
- Define source/target visibility and write authority during transfer, including post-failure reconciliation.
- Hide start button after completion/reconnect; provide alternatives for NPC death/path failure/disconnect.
- Verify TEST-PRO-001~006, 100 transfer retries with zero duplicates, and disconnect/recovery at every stage. Roughly 15 minutes is a UX assumption to refine through playtests.

### M7 — Load, faults, and operational readiness (TASK-013~014)

**TASK-013: instrumentation/scaling/long runs — depends on M5; final acceptance includes M6**

- Connect existing Tick/queue/DB/recovery metrics to Prometheus/Grafana; trace the entire command flow.
- Fix target environment for 10 players/1,000 NPCs, hardware, seed, action ratio, release-build conditions.
- Measure against Tick p95≤30ms/p99≤50ms, DB-result p95≤150ms, resync p95≤2s; record bottlenecks/remediation when missed.
- Expand from 30-minute load to 24-hour soak, then 72 hours if needed. Apply fault matrix to real processes/DB/connections.
- If adding Active/Background aggregation, separately validate asset conservation, elapsed time, and return to Active. Do not replace detailed NPC load tests with aggregate mode.
- Keep 5k/10k NPCs and 100/500 sessions as scaling experiments rather than expanding MVP acceptance unnecessarily.

**TASK-014: deployment/recovery drills — depends on TASK-008/009/013; target release completes M6**

- New-environment Docker image/Compose, TLS/WSS, secret injection, admin RBAC/audit logs.
- Health/readiness/drain plus DB-failure, Worker-recovery, and stalled-event runbooks.
- Backup/PITR restore drills, measured RPO/RTO, verified version-change/rollback boundaries.
- Create a deployment candidate bundling image digest, DB migration, rules version, and release checklist.
- Public launch/service switching is a separate deployment task. A plan is not a completed production transition.

## 5. Decisions to resolve before implementation

These are proposals, not unilateral closure of existing OPEN items. Record rationale/tests in ADRs during the owning phase.

| Decision | Initial proposal | Finalize in |
|---|---|---|
| Toolchain/libraries | Rust edition 2024; pin after compatibility checks | TASK-001 |
| RNG/IDs/hashes | Versioned RNG, recorded external input, canonical state | TASK-003 |
| protocol_version / schema_version | Separate transport and payload-schema semantics | TASK-004 |
| Command status names | Unify PENDING/PENDING_COMMIT differences in one state machine | TASK-004 |
| HP/death durability | Player death/rewards durable; temporary combat separate | Before TASK-005 |
| DB version/migrations | Isolated PostgreSQL 16+ with exact image version | TASK-007 |
| UUIDs/existing data | New world/accounts first; old data migration later | TASK-002/007 |
| Authentication/logout | Session + single-use ticket; no unauthorized autonomous movement after logout | TASK-009 |
| Game-time scale | Start at configurable 10x; validate production/consumption balance | TASK-010/011 |
| Prologue transfer | Same-DB idempotent transfer and crash reconciliation | TASK-012B |
| Hardware/budgets | State measured machine and active ratio | Before TASK-013 |
| Snapshot/event retention, RPO/RTO | Base on recovery needs and actual volume | Before TASK-014 |

For product decisions such as authentication/UX, prepare concrete alternatives and obtain user decisions at that phase. Routine environment/type/test choices proceed within the design baseline.

## 6. Work ordering and tracking

Default order: M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7.

Pure AI rules in TASK-010 may start after M2, but durable memory/economy integration completes after M3. Instrumentation/authentication/failure tests accompany their features; do not postpone them to M7. These dependencies are not instructions to run concurrent agents.

Where possible, split tickets into contracts/types→normal behavior→failure/restart→documentation/evidence. Record:

- TASK/REQ/TEST IDs and changed files.
- NOT STARTED / IN PROGRESS / BLOCKED / DONE.
- Executed commands/results, unexecuted checks and reasons.
- Schema/rules/protocol effects and recovery/rollback limits.
- Remaining work and next unblocked tickets.

Planned common Rust checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Owning tickets add DB integration/browser E2E/load commands and environments. Record golden hashes only after verified execution; never auto-update them merely to remove failures. Estimate calendar schedules after measuring TASK-001~003.

## 7. Immediate next work package

1. TASK-000: missing SQL tracking, ignores, links, new execution boundaries.
2. TASK-001: Rust Workspace, server/Runner startup, CI baseline.
3. TASK-002: minimal ECS world and StableId.
4. TASK-003: 10,000-Tick reproducibility gate.

The first demo repeats a headless scenario with matching hashes, then connects Commands/movement/combat. No new-engine implementation, testing, or deployment was performed while writing this plan.

## 8. References

- [README](../../README.en.md): first Vertical Slice, quality goals, invariants.
- [Requirements](01-requirements.md), [architecture](02-hld-architecture.md).
- [Persistence/recovery](09-persistence-recovery.md), [tests/observability](12-test-observability.md).
- [Original TASK-001~014](13-implementation-backlog.md), [operations](14-operations-security.md).
- [ADR/OPEN items](15-adrs.md), [prologue](16-prologue-gameplay.md).
- [Integration scenario](18-end-to-end-example.md).
