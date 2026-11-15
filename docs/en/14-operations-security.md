# 14. Operations, Security, Deployment, and Incident Response

[한국어](../14-operations-security.md) | **English**

## 1. Deployment

**MVP:** Docker Compose or a single server process plus PostgreSQL, with game-server Tokio runtime and Region Worker threads. Introduce Kubernetes only when needed; avoid early distributed complexity across DB, real-time Tick, and services.

**Scale-out:** stateless Gateway Deployment, stateful Region Worker domain with lease fencing, PostgreSQL HA/PITR, optional Redis only for session/token caches. The DB lease remains the source of truth for Region ownership.

## 2. Environment and configuration

| Key | Default (assumed) | Notes |
|---|---|---|
| `WSE_TICK_RATE_HZ` | 10 | Consider deterministic-version impact when changing in production |
| `WSE_GAME_TIME_SCALE` | 10 | Game time / real time |
| `WSE_REGION_CMD_CAP` | 4096 | Bounded channel |
| `WSE_DURABLE_CAP` | 512 | Reject new durable commands when full |
| `WSE_SNAPSHOT_INTERVAL_SEC` | 300 | Checkpoint interval |
| `DATABASE_URL` | Secret | Never store in plaintext config |
| `WSE_REGION_LEASE_SEC` | 15 | DB-clock based |
| `WSE_LEASE_RENEW_SEC` | 5 | Leave failure margin before expiry |
| `WSE_WS_AUTH_TIMEOUT_SEC` | 5 | Ticket authentication after WSS connection |

See [sample configuration](../../schemas/config.example.toml). Inject production secrets through environment/Secret-management tools.

## 3. Kubernetes considerations

- Readiness is true only when **Gateway and required Regions can process commands**.
- Configure graceful shutdown/terminationGracePeriod so liveness failures do not repeatedly kill Regions during persistence.
- Validate leader/lease fencing **inside DB transactions**; do not trust Kubernetes Lease alone.
- Match CPU requests/limits to active NPC counts; excessive threads cause scheduling delay.
- Prometheus metrics, trace-ID JSON logs, and datastore backups/PITR are required operational components.

## 4. Threat model

| Risk | Mitigation |
|---|---|
| Forged actor_id | Server checks token↔character ownership |
| Duplicate purchases/items | Command receipt + DB transaction |
| Client speed manipulation | Server validation/authoritative positions |
| Excess WebSocket connections | Pre-auth timeout, IP/session rate limits, maximum message size |
| Arbitrary execution through event DSL | Restricted AST/type validation; no OS/SQL calls |
| LLM prompt injection | Proposals only; server action allowlist/validation |
| Stale Worker writes | Epoch fencing/DB lease lock |
| Operations API abuse | Admin RBAC, MFA/audit logs, network restrictions |
| Secret exposure | Secret store, log masking, key rotation |
| Backup exposure | Encryption/access control/retention policy |

## 5. Incident runbooks

### DB connection failure

1. Check `wse_persistence_latency_seconds`, connection health, and Postgres alerts.
2. Stop new durable admission; retain `PENDING_COMMIT` or resolve receipts.
3. After recovery, renew lease/epoch and query receipts.
4. Resolve outstanding transactions using their existing command_id; never duplicate execution.
5. Verify backlog/Outbox and inventory/wallet invariants.

### Region Worker termination/deadlock

1. Mark Region `FAILED/PAUSED`; expose `REGION_UNAVAILABLE` through Gateway.
2. Acquire a new epoch after prior lease expiry or forced fencing.
3. Load the latest valid Snapshot; replay/reconcile durable events.
4. Verify key entity counts/assets/state hash, then activate.
5. Send `RESYNC_REQUIRED` and reconnect clients using full snapshots.

### Stalled events

1. Look up `event_id`, `correlation_id`, `tick`, and `rule_id`.
2. Distinguish unmet conditions, full queues, cooldowns, and rejected commands.
3. Apply retry/timeout/fallback without applying the same Event twice.
4. Save initial state/seed/ordered inputs for reproduction.

## 6. Backup and recovery

- Configure PostgreSQL logical/physical backup and PITR (WAL) for the environment.
- Measure actual RPO/RTO in monthly restore drills; initial RPO/RTO are **undecided**.
- Require a consistency watermark between snapshots and DB event/wallet projections.
- Backups without a restore drill do not establish production readiness.

## 7. Rollback

- Pin code by Git tag/image digest.
- Use Expand/Contract schemas; document event upcasters and downgrade limits.
- Verify whether an older version can read durable events written by a newer release before rolling back. Otherwise prepare snapshot+event conversion or a separate migration.
