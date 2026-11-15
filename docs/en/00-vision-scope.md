# 00. Vision, Game Scope, and Terminology

[한국어](../00-vision-scope.md) | **English**

## 1. Product vision

> A real-time medieval fantasy RPG where ecosystems, economies, NPC relationships, and factions evolve even while players are offline, and player choices become part of the world's permanent history.

### Game identity

- Genre: top-down/isometric 2D, sandbox, persistent world simulation.
- Core loop: **Explore → discover information/events → choose → affect the simulation → observe consequences/reputation → make another choice**.
- The appeal extends beyond predefined quests: players see the world change through player and NPC actions.
- Humans and bots/AI agents use the same server Command interface. Bot permissions and command rates have separate limits.

## 2. Example world: a frontier settlement

The initial region has 64×64 tiles and 20 residents (10 initially active detailed NPCs): 3 guards, 2 merchants, 3 farmers, 1 blacksmith, and 1 village leader; plus 5 bandits as simulation/combat participants, a forest, mine, trade route, and warehouse. These are content-design **assumptions**, not fixed values before profiling.

```text
Population grows → food consumption rises → food stocks fall
  → prices rise → more merchant caravans → more targets for bandits
  → raid/escort events → player intervention → economic/security/relationship changes
```

## 3. Initial MVP scope (keep it small)

| Included | Acceptance criterion |
|---|---|
| Single-region headless simulator | Run 10,000 ticks without a GUI and reproduce results |
| Player/NPC entities and components | Represent IDs, positions, health, needs, and inventory |
| Tile movement and pathfinding | Avoid walls/obstacles and report arrival/failure |
| Minimal combat | Melee attacks, damage, death, and loot through committed transactions |
| Basic NPC life | Farmers produce, merchants trade, guards patrol, bandits raid |
| One dynamic event | Food shortage/caravan raid occurs when conditions are met |
| Two-player shared-world observation | Server-authoritative synchronization of positions and events |
| Persistence | Atomic transaction finalization and restart recovery |
| Operational instrumentation | Tick, queue, region, and persistence metrics |

## 4. Non-goals (outside the MVP)

- Full MMO-scale distributed sharding, instant global player trading, or concurrent LLM conversations for thousands of NPCs.
- Ranked real-time PvP, complex physics, continental map generation, or a complete real-time diplomatic/political system.
- Full event sourcing with exact replay of every small movement.
- Advanced combat prediction/rollback netcode; start with interpolation and an authoritative server.

## 5. Key terms

| Term | Meaning |
|---|---|
| World | The entire persistent game, composed of Regions |
| Region | An area with independent ECS state, authority, and simulation |
| Region Worker | A computation owner executing ticks for one or more Regions |
| World Coordinator | Coordinates Region ownership, placement, and global work |
| Game Entity ID | A UUID stable across networking and DB storage; distinct from an ECS `Entity` |
| Tick | A fixed logical time step, initially 100ms |
| Command | An input expressing intent, without guaranteed success |
| Domain Event | A business fact that actually occurred under the rules |
| Delta | A message describing changes to current client-visible state |
| Durable | Assets/ownership and other state finalized by DB Commit |
| Transient | Coordinates/paths and other state allowing checkpoint-bounded loss |
| Outbox | Follow-up publication work recorded inside a DB transaction |
| Epoch/Fencing | A Region ownership generation that blocks stale Worker writes |

## 6. Validating player experience

Each event outcome must appear in at least one UI channel: **rumors, regional news, economy/security indicators, or NPC dialogue**. A choice with no feedback is a bug. Each stage has `timeout`, `fallback`, `reason_code`, and `recovery_hint` to keep the event loop moving. See [16](16-prologue-gameplay.md) for the detailed flow.
