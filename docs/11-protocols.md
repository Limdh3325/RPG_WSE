# 11. 외부 API와 프로토콜 계약 (LLD)

**한국어** | [English](en/11-protocols.md)

## 1. 일반 Envelope

모든 메시지에 `protocol_version = 1` 및 메시지 ID를 포함한다. 예시 UUID는 설명용.

```json
{
  "protocol_version": 1,
  "message_id": "aa4a99d5-610a-4a2b-a123-f1b30361d001",
  "type": "COMMAND",
  "sent_at_ms": 1794200000000,
  "payload": {}
}
```

클라이언트의 `sent_at_ms`는 표시·진단용이고 시뮬레이션 시간 결정에 사용하지 않는다.

## 2. WebSocket 메시지 타입

| 방향 | type | payload 필드 | 처리 |
|---|---|---|---|
| C→S | AUTH | ticket | 세션 인증 |
| C→S | COMMAND | command_id, client_seq, actor_id, command | 입력 수신 |
| C→S | SNAPSHOT_ACK | snapshot_id, baseline_id | 초기화 확인 |
| C→S | RESYNC_REQUEST | last_delta_seq | 전체/증분 재동기화 |
| C→S | PING | client_nonce | 네트워크 측정 |
| S→C | AUTH_OK | session_id, actor_id, region_id | 인증 완료 |
| S→C | WORLD_SNAPSHOT | baseline_id, revision, entities, region_tick | 전체 상태 |
| S→C | STATE_DELTA | baseline_id, delta_seq, changes, region_tick | 증가분 |
| S→C | COMMAND_STATUS | command_id, status, reason_code | 명령 처리 결과 |
| S→C | DOMAIN_NOTICE | event_id, event_type, text | 소문/알림 |
| S→C | RESYNC_REQUIRED | expected_seq | 상태 기준점 재설정 |
| S→C | ERROR | code, message, trace_id | 오류 |

### COMMAND 예시

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

### Snapshot 예시

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

### 상태 메시지

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

## 3. HTTP 운영/인증 API

| Method | Endpoint | Request | Response |
|---|---|---|---|
| POST | `/api/v1/auth/login` | 계정 인증 | session token |
| POST | `/api/v1/ws-tickets` | 인증 Bearer | 30초 1회용 ticket |
| GET | `/api/v1/characters` | authenticated | 사용 가능 캐릭터 |
| GET | `/api/v1/regions/:id/status` | admin only | status, epoch, tick, metrics |
| POST | `/api/v1/admin/regions/:id/pause` | admin reason | operation_id |
| POST | `/api/v1/admin/regions/:id/resume` | admin reason | operation_id |
| POST | `/api/v1/admin/regions/:id/checkpoint` | admin reason | operation_id |
| GET | `/health/live` | no auth | process liveness |
| GET | `/health/ready` | restricted as needed | DB/Region readiness |
| GET | `/metrics` | internal only | Prometheus text |

로그인 방식/계정 DB는 별도 Identity 모듈. 운영 API에는 RBAC, 감사 기록, 재확인 장치가 필요하다.

## 4. 표준 오류 코드

| 코드 | 의미 | Client 대응 |
|---|---|---|
| `UNAUTHENTICATED` | ticket/세션 없음 | 재로그인 |
| `FORBIDDEN_ACTOR` | Actor 위조 | 요청 중단 |
| `SCHEMA_UNSUPPORTED` | 프로토콜 버전 불일치 | 클라이언트 업데이트 |
| `RATE_LIMITED` | 호출량 초과 | Backoff |
| `SERVER_BUSY` | 큐 포화 | 제한 재시도 |
| `STALE_REGION_EPOCH` | 라우터 소유권 정보 오래됨 | 새 Region 라우팅 |
| `PATH_UNREACHABLE` | 경로 없음 | 목적지 수정 |
| `OUT_OF_RANGE` | 공격 거리 밖 | 이동 필요 |
| `INSUFFICIENT_FUNDS` | 금화 부족 | 사용자 안내 |
| `OUT_OF_STOCK` | 재고 부족 | 사용자 안내 |
| `IDEMPOTENCY_CONFLICT` | 같은 ID·다른 요청 | 신규 ID 재제출 전 진단 |
| `PERSISTENCE_PENDING` | Commit 결과 불명확 | receipt 확인 후 대기 |
| `RESYNC_REQUIRED` | Delta gap | Snapshot 재요청 |
| `SLOW_CONSUMER` | 연결이 서버를 따라오지 못함 | 재접속 |

## 5. 버전 관리

- 새 필드는 기본 optional + default, enum 변경 시 unknown value 처리 규칙 명시.
- `protocol_version=1`을 지원하지 않는 클라이언트는 `SCHEMA_UNSUPPORTED`.
- `domain_event.schema_version`과 `network.protocol_version`은 서로 독립.
- v1 JSON 계약은 자동 schema validator와 golden fixtures로 검증.

## 6. 보안·계약 검증

- 클라이언트의 `actor_id`는 session mapping과 일치해야 한다.
- Server는 `region_id`와 `assigned_tick`을 최종 결정한다.
- 시스템 메시지에 인증 토큰·비밀번호를 반사하지 않는다.
- 모든 메시지 최대 크기·JSON depth·타입 범위를 설정.
- `COMMAND_STATUS` 상태는 `RECEIVED/ACCEPTED/PENDING_COMMIT/SUCCEEDED/REJECTED/FAILED`; `PERSISTENCE_PENDING`은 오류로 단정하지 않고 처리 중 상태로 유지할 수 있다.
