# 03. Simulation Core and Fixed Tick (LLD)

[한국어](../03-core-tick.md) | **English**

## 1. Execution contract

- `tick_rate_hz = 10`, `fixed_delta_ms = 100` (**[Assumption]**). Configure game-time scale (`game_seconds_per_real_second=10`) separately.
- Mutations at `tick = n` follow an **explicitly defined order**.
- Do not use `Instant::elapsed()`/wall-clock time directly in simulation calculations; use it only for timeouts/operations monitoring.
- `WorldState` is a logical concept: each Region has one `bevy_ecs::World`; GlobalWorker owns global state.
- Results must not depend on incidental hash-map iteration. Sort stable UUIDs or use inputs whose order is intentionally recorded.

## 2. Tick phases

| Order | Phase | Input | Output/invariant |
|---|---|---|---|
| 0 | Admission | Gateway Commands | Assign Tick/RegionSeq; reject overflow/duplicates |
| 1 | Command Validation | Ready queue | Pending action / Rejected |
| 2 | Perception | Spatial index | Visible surroundings / danger signals |
| 3 | Navigation & Movement | Targets/speeds/tile map | Position/collision results |
| 4 | Combat | Attack input/range/HP | Damage/death/reward intents |
| 5 | NPC Decisions | Scheduled NPCs | Commands for the next Tick |
| 6 | World/Economy | Resource / Settlement | Production/consumption/prices/environment |
| 7 | Event Rules | State changes/events | Deterministically scheduled follow-up commands |
| 8 | Finalize | Transient changes, staged durable work | Deltas, durable requests, traces |
| 9 | Output | Event/Delta queues | Bounded enqueue to Replicator/Writer |

Process durable transaction ACKs with priority **before phase 0** of each Tick, preserving original durable-intent order. If ACKs can reorder, serialize in-flight work per actor/item.

## 3. Core Rust interfaces (design contracts)

```rust
pub struct TickContext {
    pub tick: u64,
    pub fixed_delta_ms: u32,
    pub game_time_ms: u64,
    pub seed: [u8; 32],
}

pub struct RegionRuntime {
    pub world: bevy_ecs::world::World,
    pub schedule: bevy_ecs::schedule::Schedule,
    pub metadata: RegionMetadata,
    // command inbox, pending transactions, outbox buffers,
    // stable_id <-> ECS Entity index, metrics
}

pub struct TickOutcome {
    pub tick: u64,
    pub transient_deltas: Vec<StateChange>,
    pub durable_intents: Vec<DurableIntent>,
    pub generated_commands: Vec<CommandEnvelope>,
}

pub trait HeadlessEngine {
    fn enqueue(&mut self, command: CommandEnvelope) -> Result<(), EngineError>;
    fn step(&mut self) -> Result<TickOutcome, EngineError>;
    fn snapshot_committed(&self) -> Result<RegionCheckpoint, EngineError>;
}
```

`TickOutcome` may contain durable intents **not yet committed in the DB**. Replicator uses transient deltas immediately; durable results are emitted separately after `PersistenceAck`.

## 4. Bevy Schedule rules

```rust
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimPhase {
    Perception, Movement, Combat, NpcDecisions, World, EventRules,
}
// schedule.configure_sets((...phase...).chain());
// schedule.set_executor(SingleThreadedExecutor::default());
```

- Systems use component queries/resources only; no IO.
- Compute each System from input state and TickContext only. Explicitly define deferred `Commands` application points.
- New NPC decisions become **next-Tick** commands. Necessary same-Tick reactions, such as damage or interrupted movement, occur only in Combat/Movement phases.
- Start with a single-thread executor; golden replay must pass before later parallelization.

## 5. Staggered NPC scheduling

- With `decision_interval_ticks=10`, normal decisions run only when `hash(stable_id) % 10 == tick % 10`.
- A separate urgent queue handles melee/damage interrupts every Tick.
- Run long-term GOAP planning every 100 Ticks or when a goal is invalidated.
- Budgets: `max_npc_evaluations_per_tick`, `max_path_expansions_per_tick`. Defer lower-priority excess work and use aging to prevent starvation.

## 6. RNG and determinism

- Pin the RNG algorithm to the engine version (for example a fixed ChaCha8 version), or build a stable hashed stream from `(world_seed, region_uuid, tick, system_id, actor_uuid, counter)`.
- No branching dependent on `HashMap` order, OS thread scheduling, or floating-point roundoff.
- Prefer integer/fixed-point coordinates, currency, and damage (`i64` millitiles, integer gold).
- Convert asynchronous LLM results into **external Commands**, logging their application Tick and payload.
- Validate output hashes using `initial snapshot + engine/rules version + ordered input log + RNG state`.

## 7. Tick scheduler pseudocode

```rust
let step = Duration::from_millis(100);
let mut deadline = Instant::now();
while running {
    drain_priority_acks_and_controls();
    collect_commands_with_admission_limit();
    let out = engine.step()?; // never await
    try_enqueue_transient(out.transient_deltas); // coalesce if full
    enqueue_or_reject_durable_safely(out.durable_intents)?;
    emit_metrics();
    deadline += step;
    if Instant::now() < deadline { sleep_until(deadline); }
    else {
        record_overrun();
        // Policy: never skip intermediate state; defer low-priority work.
        // On sustained overruns, restrict admission / enter degraded mode.
    }
}
```

**Guarantee boundary:** reproducing logical Tick results for the same inputs is a goal; actual reception times/network latency are not reproducible. Log `assigned_tick` in live mode.

## 8. Tick failure policy

| Category | Handling |
|---|---|
| Command validation failure | `REJECTED`; continue Tick |
| Data invariant violation during calculation | Region `PAUSED`; preserve evidence snapshot/log; alert operators |
| DB Writer disconnect | Block new durable intents; continue limited transient work |
| Five consecutive Tick overruns | `DEGRADED`; reduce low-priority AI work |
| Thread panic | Recover after safe lease release/expiry |

## 9. Tests

- TEST-TICK-001: headless 10,000-Tick runs with identical seed/inputs have identical state hashes.
- TEST-TICK-002: each NPC executes exactly its scheduled number of decisions.
- TEST-TICK-003: differing arrival orders reproduce according to logged sorting/assignment order.
- TEST-TICK-004: overruns do not skip Tick numbers.
- TEST-TICK-005: a 10-second asynchronous LLM delay does not increase Tick execution time.
