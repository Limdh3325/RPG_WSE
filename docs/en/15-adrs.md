# 15. Architecture Decision Records and Open Questions

[한국어](../15-adrs.md) | **English**

## 1. Decision baseline

| ADR | Status | Decision | Rationale/alternative |
|---|---|---|---|
| ADR-001 | Accepted | Rust engine/server, Bevy ECS, Tokio, Axum | ECS/memory management and IO separation; alternative: single Go server |
| ADR-002 | Accepted | ECS World per Region + single writer | Prevent state conflicts; alternative: global ECS lock |
| ADR-003 | Accepted | 10Hz fixed Tick + separate game clock | Easier validation; adjust rate after measurements |
| ADR-004 | Accepted | Initial SingleThreaded ECS schedule | Prioritize deterministic replay; parallel ECS later |
| ADR-005 | Accepted | Separate Command/Event/StateDelta | Distinguish intent, facts, and UI representation |
| ADR-006 | Accepted | No final durable success before DB Commit ACK | Prevent duplicate grants/phantom assets |
| ADR-007 | Accepted | PostgreSQL receipt + event + outbox in one TX | Atomicity/observability |
| ADR-008 | Accepted | Snapshot + durable event/reconciliation | Different recovery guarantees per state category |
| ADR-009 | Accepted | Epoch-based DB fencing | Block stale Region Worker writes |
| ADR-010 | Accepted | Utility AI + GOAP with LLM assistance | Predictable ticks and flexible language |
| ADR-011 | Accepted | MVP JSON WebSocket, Protobuf if needed | Initial development speed |
| ADR-012 | Accepted | Single process first, then Region Worker Pool | Minimize distributed complexity |
| ADR-013 | Accepted | UUID stable entity IDs | Separate from runtime ECS handles |
| ADR-014 | Accepted | Integer/fixed-point action/money/coordinate math | Better reproducibility/consistency |
| ADR-015 | Accepted | Exclude instant cross-region trade from MVP | Defer atomic multi-Region state management |

## 2. Decisions required during implementation

| ID | Topic | Current assumption | Owner/deadline (assigned by project) |
|---|---|---|---|
| OPEN-001 | Performance target/hardware | p95 30ms/1k NPCs | Tech Lead / before benchmark |
| OPEN-002 | Real-time game-time scale | 10x | Game Designer / before content |
| OPEN-003 | HP/death durability | Player death durable | Game/Server / before combat |
| OPEN-004 | Player behavior after logout | No autonomous NPC-like movement | Game Designer / before multiplayer |
| OPEN-005 | Realm/World ownership and region boundaries | One World, one Region | Architect / before transfer |
| OPEN-006 | RNG implementation/version | Fixed algorithm + stable hash | Core Engineer / before Golden Test |
| OPEN-007 | Event/snapshot retention | Long-term durable events | Operations / before production |
| OPEN-008 | Account/authentication method | HTTPS ticket + session | Security/Backend / before public WS |
| OPEN-009 | Region size/AOI cost | Region 64×64, AOI cell 16×16 | Game/Server / load testing |
| OPEN-010 | SQL migrations/DB version | PostgreSQL 16+ | DBA/Backend / before first DB application |
| OPEN-011 | Slow-client disconnect threshold | Assumed 2-second backlog | Backend / WS load testing |
| OPEN-012 | Private prologue → shared world | Idempotent transfer | Game Designer / before prologue MVP |

## 3. Change management

1. Assign a new ADR number and record *problem, options, decision, impact, rollback*.
2. Update deterministic/recovery tests for changes affecting Tick, event schema, persistence, or Region ownership.
3. Major data-boundary changes must include migrations and old-client compatibility policy.
4. Decisions may change based on benchmarks/fault tests, but document why and the consequences.

## 4. ADR template

```markdown
## ADR-XXX: Title
- Status: proposed / accepted / deprecated
- Context: Problem and scope
- Decision: Exact choice and effective point
- Alternatives: Options and why rejected
- Consequences: Performance/complexity/operations impact
- Verification: Required TEST IDs
- Migration/Rollback: Transition procedure
```
