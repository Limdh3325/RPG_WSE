# RPG World Simulation Engine (RPG-WSE)

[한국어](README.md) | **English**

> A Rust-based real-time, persistent RPG simulation engine. This is a prototype under development: design goals are distinct from implemented and integrated features.
> Stack: **Rust + Bevy ECS + Tokio + Axum + PostgreSQL + Vue 3/Phaser**.
> Target: a new real-time, persistent medieval fantasy RPG simulation. Compatibility with previous repositories is not assumed.

New contributors should read **Quick start → Repository layout → Validation → Contributing** below. Browse the [English documentation index](docs/en/README.md) or [Korean documentation index](docs/README.md).

## 0. Reading list

1. [Vision and scope](docs/en/00-vision-scope.md) — goals, MVP, non-goals, terminology.
2. [Requirements](docs/en/01-requirements.md) — functional/nonfunctional requirements and IDs.
3. [Architecture](docs/en/02-hld-architecture.md) — architecture and responsibilities.
4. [Core Tick](docs/en/03-core-tick.md) — simulation core, fixed ticks, determinism.
5. [Region/ECS](docs/en/04-region-ecs.md) — Workers, ownership, transfers.
6. [Commands/events](docs/en/05-command-event.md) — validation, ordering, deduplication.
7. [NPC AI](docs/en/06-npc-ai.md) — Utility AI, GOAP, memory, LLM boundary.
8. [World systems](docs/en/07-world-systems.md) — movement, combat, economy, factions, environment, dynamic events.
9. [Data/ERD](docs/en/08-data-erd.md) — persistent/transient data ownership.
10. [Persistence/recovery](docs/en/09-persistence-recovery.md) — transactions, outbox, snapshots, recovery.
11. [Network replication](docs/en/10-network-replication.md) — WebSocket, AOI, snapshots/deltas.
12. [Protocols](docs/en/11-protocols.md) — messages, errors, operations APIs.
13. [Testing/observability](docs/en/12-test-observability.md) — tests, benchmarks, metrics.
14. [Implementation backlog](docs/en/13-implementation-backlog.md) — dependencies and acceptance criteria.
15. [Operations/security](docs/en/14-operations-security.md) — deployment, security, disaster recovery.
16. [ADRs](docs/en/15-adrs.md) — decisions and open questions.
17. [Prologue/gameplay](docs/en/16-prologue-gameplay.md) — player experience and shared-world entry.
18. [Developer onboarding](docs/en/17-developer-onboarding.md) — contributor workflow.
19. [End-to-end example](docs/en/18-end-to-end-example.md) — purchases and economic events.
20. [Initial SQL](schemas/0001_initial.sql) — draft PostgreSQL schema.
21. [Example configuration](schemas/config.example.toml) — server settings.

## 1. Suggested reading paths

- **Planning/design:** 00 → 01 → 02 → 15 → 16.
- **Engine:** 03 → 04 → 05 → 06 → 07.
- **Data/server:** 08 → 09 → 10 → 11 → SQL.
- **QA/operations:** 12 → 13 → 14.
- **Onboarding:** this README, ADRs, relevant module documentation, then the ticket's Definition of Done.

## 2. Execution goals and priorities

**First Vertical Slice:** 64×64 frontier settlement, one player, 10 NPCs, two monster types, fixed 10Hz Tick, movement/dialogue/basic combat, food production/consumption, one dynamic event involving a merchant-caravan raid, and restart recovery. Then validate 10 concurrent players and 1,000 NPCs.

**MVP quality:** reproducible 10,000-Tick headless execution; matching Command/Event order; no duplicate durable transactions; two clients observing the same state; Tick p95 ≤30ms as an **initial target**, not a guarantee before measurement.

## 3. Implementation invariants

- Exactly **one active writer** per `region_id`; fence durable writes using `ownership_epoch`.
- Network/LLM/DB work **never blocks simulation Tick**.
- Submit all external actions as **Commands**; only Region Worker/Global Owner mutates world state.
- **Finalize durable assets, inventory, and ownership only after DB Commit/ACK**.
- Explicitly allow **checkpoint-level recovery** for transient positions/animations.
- Deterministic replay requires **engine version + initial state + RNG rules + assigned Tick/ordered inputs + recorded external results**.
- NPC LLM output is a non-authoritative proposal validated by game rules.

## 4. Documentation conventions

Use `REQ-*`, `ADR-*`, `TASK-*`, and `TEST-*` for requirements, decisions, work, and tests. **[Confirmed]** means baseline; **[Assumption]** means provisional; **[Later]** requires a separate decision. Code blocks describe contracts/examples, not a complete server or guaranteed build. Messages carry `schema_version`. In conflicts, prioritize [ADRs](docs/en/15-adrs.md) and [persistence atomicity rules](docs/en/09-persistence-recovery.md), and record the change.

Korean sources remain at `README.md` and `docs/NN-*.md`; English counterparts are `README.en.md` and `docs/en/NN-*.md`. Preserve IDs, commands, configuration values, and acceptance criteria across languages. See [maintenance instructions](docs/en/README.md).

## 5. Current implementation and limitations

This describes current source entry points/modules. Existing code does not establish integration verification or production readiness.

| Area | Current state |
|---|---|
| Headless Runner | Executes simulation from seed/Tick count; outputs JSON state hash, event count, economy summary |
| Server | Configuration validation, HTTP health, WebSocket ticket/gateway routes |
| Domain/simulation | ECS, Command/Event, movement/combat/NPC/economy/prologue code and tests |
| Persistence | PostgreSQL transaction/snapshot/recovery modules and integration tests; default server entry point does not initialize DB/full simulation |
| Web client | Vue 3 + TypeScript + Phaser UI/networking; local connection needs API/WS proxy configuration |
| Readiness | `/health/ready` currently always returns **503**; not a full-game readiness signal |

The default gateway uses `GatewayState::default()` without a simulation-command channel. Starting server and UI does not produce a fully integrated game with movement/combat/persistent storage. Default prologue file storage: `.data/prologue`.

[Development plan](docs/en/19-nmain-development-plan.md), [status](docs/en/20-development-status.md), and [M0 ADR](docs/en/21-m0-bootstrap-decisions.md) provide background. Some statements reflect historical M0/nmain policy; compare with current code/PR target. Historical `scripts/verify-m0.sh`, `scripts/sync-design-docs.py`, and `.github/workflows/wse-m0.yml` are absent. Use the individual checks below; current documentation generation uses `scripts/sync-design-docs.mjs`.

## 6. Quick start

### Prerequisites

| Tool | Purpose/baseline |
|---|---|
| Git | Clone and change management |
| Rust/Cargo | **1.90.0**, pinned in rust-toolchain.toml, with rustfmt/clippy |
| Native build tools | Visual Studio Build Tools C++ tools for Windows MSVC; platform build tools for Linux/macOS |
| Node.js/npm | Web development; exact Node version is not pinned yet, so record yours in the PR |
| Docker + Compose v2 | DB integration tests; Compose pins PostgreSQL 16.11 |
| Python 3 + websockets | Only for adapting/running existing browser E2E |

Fork and clone your Fork. Replace `<YOUR_FORK_CLONE_URL>` with the actual URL. Run in a terminal where rustup is available after Rust installation.

```sh
git clone <YOUR_FORK_CLONE_URL> RPG_WSE
cd RPG_WSE
rustup toolchain install 1.90.0 --profile minimal --component rustfmt --component clippy
cargo build --workspace --locked
```

Run all Rust commands from the **repository root**; default configuration paths are relative to the working directory.

### Server

```sh
cargo run --locked -p game-server -- --check-config
cargo run --locked -p game-server
```

Check from another terminal. Use `curl.exe` instead of `curl` in Windows PowerShell.

```sh
curl -i http://127.0.0.1:18080/health/live
curl -i http://127.0.0.1:18080/health/ready
```

Expect live **200**, ready **503**. `--check-config` validates configuration and exits; it does not verify DB connectivity/gameplay. Stop with Ctrl+C.

### Headless simulation

No DB/web client required.

```sh
cargo run --locked -p simulation-runner -- --seed 42 --ticks 10000
cargo run --locked -p simulation-runner -- --seed 42 --ticks 10000 --check
```

The first executes ticks, returning `status: completed`, `executed_ticks`, `final_state_hash`, `economy`, etc. The second only validates arguments with `executed_ticks=0`. Compare runs with identical version/initial state/seed/inputs. One final-state hash does not prove determinism of all economic processing/external systems.

### Web client

Use another terminal and the package/lock files in `apps/web-client`, not the root.

```sh
cd apps/web-client
npm ci
npm run dev -- --host 127.0.0.1
```

Open `http://127.0.0.1:3000`; if occupied, use the actual URL printed by Vite.

The client requests `/api/v1/ws-tickets` and `/ws` on the page's origin, but Vite currently has no proxy. To develop the connection, add this `proxy` inside the existing `server` object in [vite.config.ts](apps/web-client/vite.config.ts). Match target ports if changing the server listener.

```ts
proxy: {
  '/api': { target: 'http://127.0.0.1:18080', changeOrigin: true },
  '/ws': { target: 'ws://127.0.0.1:18080', ws: true, changeOrigin: true },
},
```

Restart Vite. This connects transport routes; the game-loop integration limits above still apply. Initial UI gold/inventory includes sample values.

## 7. Repository layout

| Path | Responsibility |
|---|---|
| `apps/game-server` | Server entry point, TOML/environment loader |
| `apps/simulation-runner` | Headless simulation CLI |
| `apps/web-client` | Vue/Phaser UI, network client, Vitest tests |
| `crates/game-domain` | IDs, Command/Event, rule contracts |
| `crates/ecs-components` | ECS components and Region world |
| `crates/simulation-core` | Tick execution, determinism, Worker foundation |
| `crates/simulation-systems` | Movement/combat/NPC/economy/prologue |
| `crates/persistence` | DB transactions, leases, snapshots, recovery, integration tests |
| `crates/networking` | Axum gateway, sessions, tickets, AOI, replication |
| `crates/shared-protocol` | Server/client wire protocol |
| `schemas` | Initial SQL and example server config |
| `deploy` | Development PostgreSQL Compose and environment example |
| `docs` / `docs/en` | Korean / English requirements, design, ADRs, backlog |
| `tests` | Browser E2E scripts |

Root `docker-compose.yml` is a reference for the old Go game. It points to absent `src/` paths; do not use it to start the new Rust engine.

## 8. Configuration

Server reads [schemas/config.example.toml](schemas/config.example.toml), then supported environment overrides. Select another file with `cargo run --locked -p game-server -- --config <CONFIG_PATH>`. Valid configuration does not prove all runtime modules consume it.

| Variable | Default | Meaning |
|---|---|---|
| `WSE_HTTP_LISTEN` | `127.0.0.1:18080` | HTTP/WS listener |
| `WSE_WS_AUTH_TIMEOUT_SEC` | `5` | Configured WS authentication timeout |
| `WSE_TICK_RATE_HZ` | `10` | Tick frequency |
| `WSE_GAME_TIME_SCALE` | `10` | Game-time multiplier |
| `WSE_REGION_CMD_CAP` | `4096` | Region command queue capacity |
| `WSE_DURABLE_CAP` | `512` | Durable queue capacity |
| `WSE_SNAPSHOT_INTERVAL_SEC` | `300` | Snapshot interval |
| `WSE_REGION_LEASE_SEC` | `15` | Region lease duration |
| `WSE_LEASE_RENEW_SEC` | `5` | Lease renewal interval |
| `RUST_LOG` | `info` | tracing filter |
| `PROLOGUE_DATA_DIR` | `.data/prologue` | Gateway prologue file storage |
| `DATABASE_URL` | Set explicitly | PostgreSQL integration-test connection string |

Example port override:

```sh
# Bash / zsh
WSE_HTTP_LISTEN=127.0.0.1:19080 cargo run --locked -p game-server
```

```powershell
# PowerShell: applies to the current terminal
$env:WSE_HTTP_LISTEN = '127.0.0.1:19080'
cargo run --locked -p game-server
Remove-Item Env:WSE_HTTP_LISTEN
```

`deploy/.env` is for Compose; Rust does not automatically load it. Tick frequency must divide 1000ms and cannot be lower than delta frequency. Renewal interval must be shorter than lease duration.

## 9. Development PostgreSQL

Optional for persistence work. Use a **dedicated development DB** because tests write data.

```sh
# Bash / zsh: copy only on initial setup
cp deploy/.env.example deploy/.env
```

```powershell
# PowerShell: copy only on initial setup
Copy-Item deploy/.env.example deploy/.env
```

Set your development `WSE_POSTGRES_PASSWORD` in `deploy/.env`, then:

```sh
docker compose --env-file deploy/.env -f deploy/compose.dev.yml up -d --wait
docker compose --env-file deploy/.env -f deploy/compose.dev.yml ps
```

Defaults: `127.0.0.1:55432`, user/database `wse_dev`. Set the test terminal's connection string, replacing the password placeholder and percent-encoding URI special characters.

```sh
export DATABASE_URL='postgres://wse_dev:<YOUR_DEV_PASSWORD>@127.0.0.1:55432/wse_dev'
```

```powershell
$env:DATABASE_URL = 'postgres://wse_dev:<YOUR_DEV_PASSWORD>@127.0.0.1:55432/wse_dev'
```

Compose does not automatically apply SQL. Persistence tests connect and call `apply_migration`; do not treat this as an existing-database upgrade procedure.

```sh
cargo test --locked -p persistence --test durable_transactions -- --nocapture
cargo test --locked -p persistence --test snapshot_recovery -- --nocapture
# Stop, retaining volume data
docker compose --env-file deploy/.env -f deploy/compose.dev.yml down
```

**Tests currently skip on DB connection failure and may appear successful.** Check for `Skipping PostgreSQL integration test` output and explicitly report actual DB coverage in PRs. Changing the environment password does not change an existing PostgreSQL volume's password.

## 10. Validate changes

From the repository root for Rust:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

From `apps/web-client` for frontend:

```sh
npm ci
npm test
npm run typecheck
npm run build
```

Narrow initial checks with commands such as `cargo test --locked -p simulation-core`. Report DB skips, browser checks not run, and failures separately. These are verification procedures, not a guarantee the current checkout passes.

[Browser E2E](tests/e2e_two_clients_prologue.py) resolves repository paths from the script and finds Chrome through `CHROME_BIN` or PATH. It uses CDP port 9222 and preview port 4173; adapt browser options/API/WS routing to your environment. Normal Rust/Vitest runs do not require Python.

### Troubleshooting

| Symptom | Check |
|---|---|
| Configuration file not found | Run at repository root; check `--config` path |
| Ready returns 503 | Expected currently; starting DB alone does not make it 200 |
| Server bind failure | Check port 18080; override `WSE_HTTP_LISTEN` |
| UI loads, ticket returns 404 | Vite `/api` proxy and Rust server |
| WS connection failure | `/ws` proxy `ws: true`, target port, browser Network/WS |
| DB tests succeed suspiciously quickly | Use `--nocapture`; check skips and `DATABASE_URL` |
| PowerShell blocks npm script | Invoke `npm.cmd ci`, `npm.cmd test` directly |
| `--locked` failure | Confirm intentional dependency changes and update corresponding lock file |

## 11. Contributing

Contribute bug fixes, tests, documentation, engine, or client work. Good first tasks include reproducing setup, improving DB-test skip behavior, configuring the development proxy, and improving E2E portability. Discuss large architectural changes in an Issue first.

1. Check existing Issues/PRs and the [backlog](docs/en/13-implementation-backlog.md).
2. Branch from the current default or maintainer-specified branch; do not assume historical `nmain` exists.
3. Keep changes focused on one problem with relevant tests/docs.
4. Run checks, open a PR from your Fork, and link relevant `REQ-*`, `TASK-*`, `TEST-*` IDs.

```sh
git switch -c fix/describe-the-change
# After editing/testing, stage only relevant files
git add <CHANGED_FILES>
git commit -m "Describe the change"
git push -u origin fix/describe-the-change
```

Include:

- **Problem/result:** what changes under which conditions.
- **Contracts:** API/DB/Command/Event/schema_version effects and ADR changes.
- **Validation:** commands/results, OS/tool versions, actual PostgreSQL coverage.
- **Limits:** unrun tests, known issues, compatibility/recovery effects.
- **UI:** before/after screenshots and browser checks.

Follow rustfmt/clippy. Update Cargo.lock/web package-lock.json with dependency changes; avoid unrelated lock churn. Check both Rust and TypeScript for protocol changes. Record architecture rationale/impact in [ADRs](docs/en/15-adrs.md) and preserve single-writer/nonblocking-Tick/Commit-ACK invariants.

Bug Issues need reproduction, expected/actual results, commit, OS/tool versions, and sanitized logs; determinism bugs also need seed/Ticks/inputs. Never publish passwords, tickets, real account data, or private connection strings. Use private vulnerability reporting when the repository offers it.

## 12. License

Root [LICENSE](LICENSE) is **Apache License 2.0**. Dependencies/assets retain their respective licenses. Record provenance, license, and redistribution terms when adding assets. Asset notes in the [client README](apps/web-client/README.md) do not replace the project license.
