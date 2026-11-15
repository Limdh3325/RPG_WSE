# M0 Development Environment Decisions

[한국어](../21-m0-bootstrap-decisions.md) | **English**

Status: M0 implementation decisions. ADR-001~015 remain unchanged. Runtime statements below describe M0; use the root README for the current implementation.

## ADR-016: New execution boundary and pinned tools

- Pin Rust 1.90.0, edition 2024, Cargo resolver 3.
- Pin Axum 0.8.6 / Tokio 1.48.0 / Bevy ECS 0.17.2 / SQLx 0.8.6 and direct dependencies exactly; reproduce transitive dependencies with Cargo.lock.
- Prioritize compile/test evidence for this combination over chasing newest versions. Validate upgrades as separate changes.
- game-domain/simulation crates have no networking/SQLx dependencies; persistence/networking are separate crates.
- Server starts with live=200 and ready=503. Do not claim production readiness for M0 without Regions/DB.
- M0 Runner succeeds only with `--check` and reports executed_ticks=0; actual execution returns not-implemented.
- Verify using an isolated Rust Docker image on the target Linux server rather than installing Windows tools.
- New development DB uses separate Compose project/DB/volume, bound only to 127.0.0.1:55432.
- Keep the old root Compose as reference. Do not restart old services or migrate old data.

## Configuration and secrets boundary

- Read the specified TOML, apply supported WSE_* overrides, then validate.
- Missing files, unknown keys, invalid numbers, and unsupported protocols fail startup.
- Supported: WSE_HTTP_LISTEN, WSE_WS_AUTH_TIMEOUT_SEC, WSE_TICK_RATE_HZ, WSE_GAME_TIME_SCALE, WSE_REGION_CMD_CAP, WSE_DURABLE_CAP, WSE_SNAPSHOT_INTERVAL_SEC, WSE_REGION_LEASE_SEC, WSE_LEASE_RENEW_SEC.
- Other settings stay in TOML. DATABASE_URL begins in TASK-007; M0 neither logs it nor connects.
- Default development listener: 127.0.0.1:18080.
- Health endpoints expose process status only, never game data/secrets.

## Documentation editing and validation

Source documents are README.md, README.en.md, docs/NN-*.md, docs/en/NN-*.md, and schemas. Follow the [documentation maintenance guide](README.md) to regenerate integrated documents and manifest; do not edit generated artifacts directly.

The historical M0 commands were `python3 scripts/sync-design-docs.py` and its `--check` variant. That script is absent from this checkout; use the current Node.js workflow in the guide.

Manifest records source bytes/hashes rather than generation time so identical sources produce identical results. SQL is a design draft until validated against a new DB; tracking it in Git is distinct from DB execution verification.

Historical CI policy targeted nmain PRs and nmain/codex branch pushes. Defining a workflow and passing remotely hosted CI are separate statuses.
