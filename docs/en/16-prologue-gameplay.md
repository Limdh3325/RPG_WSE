# 16. Player Experience, Prologue, and Shared-World Entry

[한국어](../16-prologue-gameplay.md) | **English**

## 1. Purpose

Even in a simulation-centered game, players must understand what to do in the first 15 minutes. The private prologue introduces core features in sequence; the later shared world centers on autonomous events.

## 2. Fifteen-minute prologue (initial assumption)

| Stage | Player action | Learning/validation | Failure/alternative |
|---|---|---|---|
| P0 | Enter settlement, talk to NPC | Movement/interaction | Retry/highlight failed dialogue |
| P1 | Gather resources/help NPC | Ownership/inventory | Storage/recovery when capacity is full |
| P2 | Travel to frontier outpost | Automatic pathfinding | Repath/alternate route if blocked |
| P3 | Rescue Kael/fight goblins | Combat/damage/relationships | Recoverable state after defeat/departure |
| P4 | Collect supplies and migrate | Transactions/resources/events | Automatic prerequisite-event recovery checks |
| P5 | Reach shared settlement | Private→shared world transfer | Requery transfer receipt on timeout |

## 3. Prologue state machine

`NOT_STARTED → IN_PROGRESS → READY_TO_TRANSFER → TRANSFERRING → COMPLETED`. Persist transitions, `completed_at`, `transfer_id`, and `reward_receipts` durably in the DB.

- Every event has `prerequisites`, `completion`, `timeout`, `fallback`, and `diagnostics`.
- Gate only essential learning features; do not force every player choice down one route.
- Define recovery paths so `READY_TO_TRANSFER` does not depend solely on **one prerequisite event**, preventing stalls after Kael's rescue.
- Once the server finalizes `COMPLETED`, **hide the private-prologue start button**. Render from server state, not local UI state alone.
- `transfer_id` retries are idempotent; never duplicate NPCs/items/characters in the shared world.

## 4. UI/game-state contracts

| UI | Server-owned state | Requirement |
|---|---|---|
| Auto-move button | Navigation state | Show received/pathfinding/moving/completed/failed |
| Dialogue choices | Interaction availability | Revalidate distance/permission/dialogue session |
| Supply acquisition | Durable Inventory receipt | Do not finalize acquisition before success ACK |
| Prologue start button | Prologue state | Hide when COMPLETED |
| World news | Confirmed domain events | Never present uncommitted events as final facts |

## 5. Dynamic situations that sustain interest

After the prologue, provide `Food shortage → Caravan ambush → Guard call → Player/NPC intervention`. NPCs create outcomes even without player intervention, changing shops/security/relationships. Make world changes visible through local news, NPC dialogue, and prices.

## 6. Tests and DoD

- TEST-PRO-001: new account starts/completes prologue and enters shared world.
- TEST-PRO-002: continue after disconnect/reconnect at every event stage.
- TEST-PRO-003: retry the same transfer 100 times after READY_TO_TRANSFER; zero duplicate characters/items.
- TEST-PRO-004: hide the prologue button on a new connection after completion.
- TEST-PRO-005: show auto-move failure state/reason within 5 seconds (initial UX target).
- TEST-PRO-006: path/combat failure or NPC death never permanently blocks the required tutorial.
