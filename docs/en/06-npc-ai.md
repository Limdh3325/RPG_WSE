# 06. NPC Decision Engine — Utility AI + GOAP (LLD)

[한국어](../06-npc-ai.md) | **English**

## 1. Design goals

NPCs act according to goals and needs even without players. Separate **Perception → Urgent Reaction → Utility Goal Selection → GOAP Planning → Action Execution**, rather than replanning every NPC every Tick. NPC AI never directly mutates durable state; it submits an `NPC_ACTION` Command for the next Tick.

## 2. NPC data model

| Component | Fields | Storage/update |
|---|---|---|
| Identity | stable_id, profession, home_region, faction_id | Persistent DB |
| Needs | hunger, fatigue, safety, wealth_pressure (0..1000) | ECS/checkpoint |
| Personality | courage, greed, loyalty, curiosity (-1000..1000) | Persistent DB |
| Relationship | target_id, trust, affinity, fear, debt | DB for important changes |
| Memory | kind, target, severity, happened_tick, ttl, immutable refs | Important memories in DB; short-term in ECS |
| Goals | survive, eat, earn, defend, revenge | AI state/checkpoint |
| Plan | GOAP action list, current_index, precondition_hash | ECS/checkpoint |
| AI Schedule | next_decision_tick, last_replan_tick | ECS/checkpoint |

## 3. Perception: use only known facts

- Query entities within `vision_range_tiles` through a spatial hash/grid.
- Generate **observed Facts** such as `VisibleEntity`, `Threat`, `AvailableFood`, `NearbyWork`, `FactionAlert`, and `KnownRoute`.
- Do not read another Region's internal state as if by telepathy. Use communicated facts: rumors, messengers, or world notices.
- Terrain/fog/occlusion is P1; the MVP uses radius and simple obstacle checks.

## 4. Urgent reactions and normal decisions

**Urgent reactions:** damage, attacks, stun, fire. Enqueue each Tick or on an event; evaluate `DODGE`, `FLEE`, `CALL_GUARD`, and `COUNTERATTACK`. Apply cooldown after processing.

**Normal decisions:** initially once per second, staggered by a stable NPC UUID hash. Retain valid ongoing actions when there is no emergency; avoid changing goals every evaluation (hysteresis).

## 5. Utility scoring

Each goal `g` produces a utility in 0–1000 from input facts.

\[
 U(g)=\operatorname{clamp}_{[0,1000]}(\sum_i w_{g,i}\,c_i(facts, needs) + p_g(personality) + q_g(context) - k_g(cooldown))
\]

- Normalize `c_i` to integers in 0–1000; use fixed-point `w` (for example per-mille) and 64-bit intermediate dot products.
- Store all weights/curves in versioned data files.
- Retain the current goal if the score difference is at most `switch_margin=100` to prevent oscillation.
- Break ties by `(goal_priority, stable_goal_id)`.

**Example NPC:** `hunger=850`, `fatigue=400`, `threat=700`, `food_available=true`.

| Candidate | Inputs/context | Example utility | Result |
|---|---|---:|---|
| EAT | High hunger, accessible food | 830 | Candidate |
| REST | Moderate fatigue, accessible home | 410 | Candidate |
| FLEE | High threat, low courage | 930 | **Selected** |
| WORK | Workplace available but dangerous | 180 | Candidate |

Values are examples and belong in external balance data. Selecting `FLEE` passes the `Safe=true` goal to GOAP.

## 6. GOAP planner

### Example World Facts

```rust
struct Facts {
    has_food: bool,
    near_home: bool,
    is_safe: bool,
    has_gold: bool,
    shop_open: bool,
}

struct GoapAction {
    id: &'static str,
    preconditions: Vec<FactPredicate>,
    effects: Vec<FactEffect>,
    cost_milli: u32,
    max_duration_ticks: u32,
}
```

### Basic actions

| Action | Precondition | Effect | Example cost |
|---|---|---|---:|
| MOVE_TO_MARKET | route_available, not_threatened | at_market=true | path_length×10 |
| BUY_FOOD | at_market, has_gold, stock>0 | has_food=true | price + queue |
| EAT_FOOD | has_food, not_in_combat | hunger_reduced=true | 200 |
| FLEE_TO_GUARD | threat_present, route_available | is_safe=true | Route risk |
| REST_AT_HOME | near_home, safe | fatigue_reduced=true | 500 |
| WORK_FIELD | season_ok, safe, tools_ok | earned_food=true | 700 |

### Search algorithm

1. Select Goal Facts with Utility AI.
2. Load current NPC Facts and the Action Catalog.
3. Expand feasible precondition/effect nodes using A* or Dijkstra.
4. Limit `max_expansions=128`, `max_plan_length=12`, and `max_plan_ms`. Wall-clock time is only for budgets/operations, never result ordering.
5. Action cost is an integer-weighted sum of `duration + risk + resource + social_cost`.
6. Break ties by stable action ID. On search failure use `fallback = REST/WAIT/FLEE`.
7. Submit execution as a next-Tick command; the server revalidates actual costs/stock.

### Replanning triggers

- Goal reached, combat/threat changed, action failed, required materials exhausted, path blocked, or plan expired.
- After three consecutive failures: exponential cooldown, plan diagnostics, and safe Idle.

## 7. GOAP execution state machine

`IDLE → EVALUATING → PLANNING → EXECUTING → (SUCCESS | FAILED | INTERRUPTED) → EVALUATING`.

Each Action records `action_id`, `entity_id`, `plan_version`, `started_tick`, `timeout_tick`, `target_id`, and `status`. Safely cancel on timeout or stale `plan_version`. Repeated action failures emit an `NPC_STUCK` diagnostic event.

## 8. NPC memory and relationships

- Short-term: observed entities, recent dialogue, current threats; expire by game-time TTL.
- Long-term: important rescues, betrayals, murders, rewards; persist in DB.
- Store trust/affinity/fear as integers keyed by `(subject_id, target_id)`.
- Submit follow-up updates as `RelationshipChangeCommand`; update ECS projection after DB Commit.
- Give important memories unique `source_event_id` references to prevent learning the same event twice.

## 9. LLM boundary

```
Dialogue request (async) -> NPC facts + policy-approved memory
  -> LLM proposal (text, optional suggested_action)
  -> schema parse / content policy / rule validation
  -> dialogue broadcast or Command queued in future Tick
```

- LLMs cannot modify actual NPC asset/item/location databases.
- Revalidate proposed actions for permission, feasibility, resources, relationships, and distance.
- On timeout, fall back to template dialogue/Utility decisions. Check response idempotency key and requested world revision.
- Do not store personal data/raw conversations unchanged as long-term memory; retain only purpose-appropriate summaries under retention policy.

## 10. Decision performance

- A target of `1,000 NPCs / one decision per second` averages about 100 NPC evaluations per 10Hz Tick.
- Use a spatial index for Perception; fully replan GOAP only when goals change.
- Instrument `npc_decisions_total`, `goap_expansions_total`, `npc_plan_failures_total`, and `npc_stuck_total`.

## 11. Tests and acceptance

- TEST-AI-001: urgent threats prioritize FLEE over EAT despite high hunger.
- TEST-AI-002: without food, available gold and a shop produce a BUY → EAT plan.
- TEST-AI-003: invalid paths trigger replanning; repeated failures cause Idle and diagnostics.
- TEST-AI-004: identical inputs/seed/facts produce identical goals/plans.
- TEST-AI-005: an LLM proposal to grant a nonexistent item results in zero grants.
- TEST-AI-006: no duplicate important-memory application for a source_event_id.
