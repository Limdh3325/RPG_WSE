# 11. External API and Protocol Contracts (LLD)

[한국어](../11-protocols.md) | **English**

## 1. General envelope

All messages include `protocol_version = 1` and a message ID. UUIDs below are illustrative.

```json
{
  "protocol_version": 1,
  "message_id": "aa4a99d5-610a-4a2b-a123-f1b30361d001",
  "type": "COMMAND",
  "sent_at_ms": 1794200000000,
  "payload": {}
}
```

Client `sent_at_ms` is for display/diagnostics, never simulation-time decisions.

## 2. WebSocket message types

| Direction | type | Payload fields | Handling |
|---|---|---|---|
| C→S | AUTH | ticket | Authenticate session |
| C→S | COMMAND | command_id, client_seq, actor_id, command | Receive input |
| C→S | SNAPSHOT_ACK | snapshot_id, baseline_id | Confirm initialization |
| C→S | RESYNC_REQUEST | last_delta_seq | Full/incremental resync |
| C→S | PING | client_nonce | Measure network |
| S→C | AUTH_OK | session_id, actor_id, region_id | Authentication complete |
| S→C | WORLD_SNAPSHOT | baseline_id, revision, entities, region_tick | Full state |
| S→C | STATE_DELTA | baseline_id, delta_seq, changes, region_tick | Incremental state |
| S→C | COMMAND_STATUS | command_id, status, reason_code | Command result |
| S→C | DOMAIN_NOTICE | event_id, event_type, text | Rumors/notices |
| S→C | RESYNC_REQUIRED | expected_seq | Reset state baseline |
| S→C | ERROR | code, message, trace_id | Error |

### COMMAND example

```json
{
  "protocol_version": 1,
  "message_id": "aa4a99d5-610a-4a2b-a123-f1b30361d002",
  "type": "COMMAND",
  "payload": {
    "command_id": "fcdb3443-2693-4406-8b05-a9c476150f63",
    "client_seq": 17,
    "actor_id": "77bdc378-734a-45d1-ae65-390333b5ce14",
    "command": {"kind":"MOVE_TO", "x_millitile":14000, "y_millitile":12000}
  }
}
```

### Snapshot example

```json
{
  "protocol_version": 1,
  "message_id": "aa4a99d5-610a-4a2b-a123-f1b30361d003",
  "type": "WORLD_SNAPSHOT",
  "payload": {
    "world_id": "f2b8bf9a-4070-421f-b8a2-ae05e18a4041",
    "region_id": "8231eeec-2bf0-4a87-8671-d61016828889",
    "baseline_id": "baseline-42",
    "revision": 4201,
    "region_tick": 123456,
    "entities": [
      {"id":"77bdc378-734a-45d1-ae65-390333b5ce14","kind":"PLAYER", "position":{"x":14000,"y":12000},"hp":100}
    ]
  }
}
```

### Status message

```json
{
  "type": "COMMAND_STATUS", "protocol_version": 1,
  "message_id": "aa4a99d5-610a-4a2b-a123-f1b30361d004",
  "payload": {
    "command_id": "fcdb3443-2693-4406-8b05-a9c476150f63",
    "status": "REJECTED", "reason_code": "PATH_UNREACHABLE",
    "region_tick": 123457
  }
}
```

## 3. HTTP operations/authentication APIs

| Method | Endpoint | Request | Response |
|---|---|---|---|
| POST | `/api/v1/auth/login` | Account authentication | Session token |
| POST | `/api/v1/ws-tickets` | Authenticated Bearer | Single-use 30-second ticket |
| GET | `/api/v1/characters` | Authenticated | Available characters |
| GET | `/api/v1/regions/:id/status` | Admin only | status, epoch, tick, metrics |
| POST | `/api/v1/admin/regions/:id/pause` | Admin reason | operation_id |
| POST | `/api/v1/admin/regions/:id/resume` | Admin reason | operation_id |
| POST | `/api/v1/admin/regions/:id/checkpoint` | Admin reason | operation_id |
| GET | `/health/live` | No auth | Process liveness |
| GET | `/health/ready` | Restricted as needed | DB/Region readiness |
| GET | `/metrics` | Internal only | Prometheus text |

Login/account storage belongs to a separate Identity module. Operations APIs require RBAC, audit records, and confirmation safeguards.

## 4. Standard error codes

| Code | Meaning | Client response |
|---|---|---|
| `UNAUTHENTICATED` | No ticket/session | Log in again |
| `FORBIDDEN_ACTOR` | Forged Actor | Stop request |
| `SCHEMA_UNSUPPORTED` | Protocol mismatch | Update client |
| `RATE_LIMITED` | Excess request rate | Backoff |
| `SERVER_BUSY` | Queue full | Bounded retry |
| `STALE_REGION_EPOCH` | Stale router ownership | Route to current Region owner |
| `PATH_UNREACHABLE` | No path | Change destination |
| `OUT_OF_RANGE` | Outside attack range | Move closer |
| `INSUFFICIENT_FUNDS` | Insufficient gold | Inform user |
| `OUT_OF_STOCK` | Insufficient stock | Inform user |
| `IDEMPOTENCY_CONFLICT` | Same ID, different request | Diagnose before resubmitting with a new ID |
| `PERSISTENCE_PENDING` | Unresolved Commit result | Check receipt and wait |
| `RESYNC_REQUIRED` | Delta gap | Request Snapshot |
| `SLOW_CONSUMER` | Connection cannot keep up | Reconnect |

## 5. Versioning

- New fields default to optional with defaults; define unknown-value handling for enum changes.
- Clients without `protocol_version=1` support receive `SCHEMA_UNSUPPORTED`.
- `domain_event.schema_version` and `network.protocol_version` are independent.
- Validate v1 JSON contracts with automated schema validators and golden fixtures.

## 6. Security and contract checks

- Client `actor_id` must match session mapping.
- Server makes final `region_id` and `assigned_tick` decisions.
- Never echo authentication tokens/passwords in system messages.
- Set maximum message size, JSON depth, and type ranges.
- `COMMAND_STATUS` states: `RECEIVED/ACCEPTED/PENDING_COMMIT/SUCCEEDED/REJECTED/FAILED`. `PERSISTENCE_PENDING` may remain in progress rather than being classified as failure.
