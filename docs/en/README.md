# English Documentation

[한국어](../README.md) | **English**

Read the [project README](../../README.en.md) first for setup and current limitations. Documents 00–21 contain design contracts and historical milestone records; planned features are not claims of implementation.

| Document | Topic |
|---|---|
| [00-vision-scope.md](00-vision-scope.md) | 00. Vision, Game Scope, and Terminology |
| [01-requirements.md](01-requirements.md) | 01. Software Requirements Specification (SRS) |
| [02-hld-architecture.md](02-hld-architecture.md) | 02. Overall Architecture (HLD) |
| [03-core-tick.md](03-core-tick.md) | 03. Simulation Core and Fixed Tick (LLD) |
| [04-region-ecs.md](04-region-ecs.md) | 04. Region Worker, ECS, and Ownership (LLD) |
| [05-command-event.md](05-command-event.md) | 05. Command/Event Pipeline and Execution Rules (LLD) |
| [06-npc-ai.md](06-npc-ai.md) | 06. NPC Decision Engine — Utility AI + GOAP (LLD) |
| [07-world-systems.md](07-world-systems.md) | 07. World Systems — Movement, Combat, Economy, Factions, Environment, Events (LLD) |
| [08-data-erd.md](08-data-erd.md) | 08. Persistent Data Model and ERD (LLD) |
| [09-persistence-recovery.md](09-persistence-recovery.md) | 09. Persistence, Transactions, Snapshots, and Recovery (LLD) |
| [10-network-replication.md](10-network-replication.md) | 10. Multiplayer Synchronization and Network/AOI (LLD) |
| [11-protocols.md](11-protocols.md) | 11. External API and Protocol Contracts (LLD) |
| [12-test-observability.md](12-test-observability.md) | 12. Test Strategy, Benchmarks, and Observability |
| [13-implementation-backlog.md](13-implementation-backlog.md) | 13. Implementation Backlog and Work Instructions |
| [14-operations-security.md](14-operations-security.md) | 14. Operations, Security, Deployment, and Incident Response |
| [15-adrs.md](15-adrs.md) | 15. Architecture Decision Records and Open Questions |
| [16-prologue-gameplay.md](16-prologue-gameplay.md) | 16. Player Experience, Prologue, and Shared-World Entry |
| [17-developer-onboarding.md](17-developer-onboarding.md) | 17. Developer Onboarding and Getting Started |
| [18-end-to-end-example.md](18-end-to-end-example.md) | 18. End-to-End Implementation Example — Purchases and Food Shortages |
| [19-nmain-development-plan.md](19-nmain-development-plan.md) | Detailed Development Plan for the New nmain Engine |
| [20-development-status.md](20-development-status.md) | Development Status and Requirements Traceability |
| [21-m0-bootstrap-decisions.md](21-m0-bootstrap-decisions.md) | M0 Development Environment Decisions |
| [22-agent-work-measurement.md](22-agent-work-measurement.md) | Agent Work Measurement |

## Maintaining both languages

Track current work in the [work index](../work-index.md). The index and inbox/outbox request/result records are shared Korean operational records. Both language versions of the measurement rules link to the same records; per-task translations are not required.

- Korean: root README.md and docs/NN-*.md. English: root README.en.md and docs/en/NN-*.md.
- Preserve filenames, REQ/ADR/TASK/TEST IDs, numeric limits, commands, protocol fields, and acceptance criteria. Translate prose and comments, not executable identifiers.
- Update both counterparts in the same change and retain language-switch links. Do not translate historical status into a claim of current completion.
- Keep code/schema files shared. Integrated documents embed the original SQL/TOML, including their original comments.
- From the repository root, use Node.js (no additional packages):

```sh
node scripts/sync-design-docs.mjs
node scripts/sync-design-docs.mjs --check
```

This regenerates RPG_WSE_Complete_Design.md, RPG_WSE_Complete_Design.en.md, and manifest.json, then checks language pairs, local links, code fences, required IDs, and generated-file freshness. The check does not execute application examples or prove semantic translation equivalence. Do not edit generated artifacts directly.
