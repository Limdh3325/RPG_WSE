# 04. Region Worker, ECS, and Ownership (LLD)

[한국어](../04-region-ecs.md) | **English**

## 1. Region ownership model

- Each Region has exactly **one Active Owner**.
- PostgreSQL stores `region_leases(region_id, owner_id, ownership_epoch, lease_expires_at)`.
- Every **durable DB write transaction** locks the lease row and checks `epoch = expected_epoch` and `lease_expires_at > clock_timestamp()` before proceeding.
- Assigning a new Epoch rejects old Worker commits even if that Worker remains alive after GC/scheduling delays.
- Gateway routing caches include the epoch; refresh and reroute commands on mismatch.

## 2. ECS World structure

```
RegionRuntime
  ├── RegionMetadata { region_id, epoch, status, tick }
  ├── bevy_ecs::World
  │   ├── Entity(opaque Bevy handle) -> StableEntityId(UUID)
  │   ├── Position, Velocity, Navigation, Collider
  │   ├── Health, CombatState, InventoryProjection
  │   ├── Needs, Personality, AIState, MemoryRef
  │   └── Resources: Clock, MapGrid, SpatialIndex, Rng
  ├── Schedule (ordered systems)
  ├── stable_entity_index: UUID -> Bevy Entity
  ├── command_queue / priority_ack_queue
  ├── pending_durable: command_id -> reservations
  └── changed_entity_set / tick_metrics
```

A Bevy `Entity` can change after Region restart; never persist it directly in network messages, DB records, or logs. Use `StableEntityId(Uuid)` and rebuild the index on recovery.

## 3. Required components

| Component | Example fields | Validation |
|---|---|---|
| StableId | UUID | Unique within Region |
| Position | `x_millitile:i64, y_millitile:i64` | Valid terrain/region coordinates |
| Velocity | `dx,dy:i32` | Movement speed/ability cap |
| Navigation | target, path, path_version, state | Valid path/state machine |
| Health | current,max:i32 | 0 ≤ current ≤ max |
| Needs | hunger,fatigue,safety:u16 | Clamp to 0..1000 |
| Personality | courage,greed,loyalty:i16 | -1000..1000 |
| AIState | goal,action,next_decision_tick | Valid plan/cooldown |
| InventoryProjection | committed_version, slots | Apply only DB-committed results |
| FactionMembership | faction_uuid,standing | Validate GlobalWorker version |

## 4. Region Worker thread

```rust
struct RegionHandle {
    id: uuid::Uuid,
    epoch: u64,
    cmd_tx: tokio::sync::mpsc::Sender<RegionMessage>,
}

enum RegionMessage {
    Command(CommandEnvelope),
    PersistenceAck(PersistenceAck),
    GlobalStateUpdate(GlobalProjection),
    SetSimulationMode(SimulationMode),
    Shutdown,
}
```

Tokio networking uses `Sender::try_send`/`send` as appropriate. Dedicated Region OS threads collect bounded inputs with `Receiver::try_recv` before each Tick. Successful `try_send` means **queued**, not executed successfully or committed to the DB.

### Queue policies

| Queue | Provisional cap (assumption) | When full |
|---|---:|---|
| Player Command / Region | 4096 | Return `SERVER_BUSY`; per-session rate limit |
| Persistence Ack | 1024 | Guaranteed priority/separate path; stop safely |
| Transient Delta | 2048 | Coalesce positions/drop old deltas, then snapshot |
| Durable Intent | 512 | Reject new durable commands; retain in-flight reservations |
| Control | 64 | Priority service; health fault if unavailable |

## 5. Region lifecycle

`LOADING → ACTIVE/BACKGROUND → PAUSED/MIGRATING → STOPPED`. Track `health = HEALTHY/DEGRADED/FAILED` separately; do not mix lifecycle and health.

- LOADING: acquire Epoch → validate Snapshot/event/durable Projection consistency → rebuild spatial index.
- ACTIVE: player-present region with detailed movement/combat/AI.
- BACKGROUND: omit some individual real-time NPC movement and aggregate production/consumption; preserve important NPC states.
- MIGRATING: stop new Command admission, drain/finalize durable work, checkpoint safely, hand over Epoch, load target Worker, then change routing.
- PAUSED: administrative/abnormal state; explicitly reject or queue external inputs.

## 6. Inter-region movement (single-server MVP)

```mermaid
sequenceDiagram
  participant S as Source Region
  participant C as Transfer Coordinator
  participant D as Durable Store
  participant T as Target Region
  S->>C: TransferRequest(entity, source_epoch)
  C->>S: Mark TRANSFERRING; stop local commands
  C->>D: transfer record unique transfer_id / fence epochs
  D-->>C: committed transfer intent
  C->>T: spawn frozen entity snapshot
  T-->>C: target ready
  C->>D: commit new region_id/location
  D-->>C: committed
  C->>S: despawn source
  C->>T: activate entity
```

Restart failed transfers using the `transfer_id` receipt. Target creation is an idempotent upsert; source and target must never be active simultaneously. Initially support only `Player` transfers; arbitrary NPC transfers follow later.

## 7. Global-state principles

- A separate GlobalWorker is the single writer for Kingdom/Faction/World Clock/macroeconomic state.
- Regions use read-only Snapshots containing `global_projection_version`.
- Send global commands to GlobalWorker and apply resulting Events asynchronously to Regions.
- Multi-region transactions requiring consistency are outside the MVP. Eventually consistent events still require versioning and deduplication.

## 8. Tests

- TEST-REGION-001: only one of two Workers with the same Region Epoch can commit.
- TEST-REGION-002: recover with a new Epoch after Worker panic; reject stale writes.
- TEST-REGION-003: no cross-mutation of ECS state between two concurrent Regions.
- TEST-REGION-004: NPC/stock-total invariants survive BACKGROUND↔ACTIVE transitions.
- TEST-REGION-005: termination at any transfer stage creates zero duplicate characters.
