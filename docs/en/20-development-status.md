# Development Status and Requirements Traceability

[한국어](../20-development-status.md) | **English**

Baseline: the new nmain engine. Test results from the previous game are not evidence that the new engine is complete. This table preserves the source document's recorded status; consult the root README and current implementation for subsequent changes.

| TASK | REQ | Verification | Status |
|---|---|---|---|
| 000 Repository | N-008 | SQL/config tracked, documentation links/manifest check | DONE (M0 local gate) |
| 001 Environment | F-001 foundation, N-007 foundation | fmt/clippy/test/build, actual HTTP/CLI smoke, CI definition | DONE (M0 local gate) |
| 002 ECS | F-002 | Stable ID creation/deletion/restore, 10,000-Entity lifecycle, duplicate rejection | DONE (local gate passed) |
| 003 Tick | F-001/003, N-001 | 10,000-Tick state/event hashes | DONE (local gate passed) |
| 004 Contracts | F-004/009, N-008 | Duplicates/order/version/rule cycles | DONE (Local gate passed) |
| 005 Movement/combat | F-007/008/015 | Collision/paths/range/death | DONE (Local gate passed) |
| 006 Worker | F-005/017, N-005 | Single writer, bounded queues, shutdown | NOT STARTED |
| 007 Durable transactions | F-004/005/010, N-004/009 | Real PG, Commit/ACK failpoints | NOT STARTED |
| 008 Recovery | F-010, N-009 | Checksum/restart/receipt/RNG | NOT STARTED |
| 009 Networking | F-011, N-006/008 | Two clients/forgery/gaps/reconnect | NOT STARTED |
| 010 NPCs | F-006/014 | Autonomous life/planning-failure recovery | NOT STARTED |
| 011 Economy | F-009/013/015 | Shortage→caravan→raid→economic outcomes | NOT STARTED |
| 012A UI | F-007/008/011 | Real-browser E2E | NOT STARTED |
| 012B Prologue | F-016 | TEST-PRO-001~006 | NOT STARTED |
| 013 Instrumentation/load | F-012, N-002/003/005/007 | Metrics/load/soak/Background consistency | NOT STARTED |
| 014 Operations | F-017, N-006/009 | Backup restore/deployment/runbook | NOT STARTED |
| Later LLM | F-018 | Non-authoritative proposals, timeout/fallback | DEFERRED |
| Later distribution | F-019 | Multi-server Region transfer | DEFERRED |

F/N prefixes refer to REQ-F/REQ-N in doc 01. Owning TASKs link concrete TEST IDs.
M0 Runner `--check` validates CLI only; it does not complete REQ-F-001 headless simulation.
M0 must not expose fabricated Tick/persistence metrics.

Historical completion evidence: `outbox/2026-11-09-m0-completion.md` (not included in this checkout). GitHub-hosted CI was NOT RUN at this recorded pre-commit/push point.
