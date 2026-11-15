# 14. 운영 · 보안 · 배포 · 장애 대응

**한국어** | [English](en/14-operations-security.md)

## 1. 배포 방식

**MVP:** Docker Compose 또는 단일 서버 프로세스 + PostgreSQL, 개별 game-server Tokio runtime과 Region Worker threads. K8s는 필요 시 도입; 초기에 DB/실시간 Tick/여러 서비스 간 분산 복잡도 증가를 피한다.

**확장:** Gateway Deployment(stateless), Region Worker Stateful domain(lease fencing), PostgreSQL HA/PITR, Redis는 session/token cache에만 선택 도입. Region ownership의 source-of-truth는 DB lease.

## 2. 환경 변수 / 설정

| 키 | 기본(가정) | 비고 |
|---|---|---|
| `WSE_TICK_RATE_HZ` | 10 | 운영에서 변경 시 deterministic version 주의 |
| `WSE_GAME_TIME_SCALE` | 10 | 게임 시간 / 현실 시간 |
| `WSE_REGION_CMD_CAP` | 4096 | bounded channel |
| `WSE_DURABLE_CAP` | 512 | 채널이 차면 신규 durable 명령 거부 |
| `WSE_SNAPSHOT_INTERVAL_SEC` | 300 | checkpoint 주기 |
| `DATABASE_URL` | secret | 평문 config에 저장 금지 |
| `WSE_REGION_LEASE_SEC` | 15 | DB clock 기준 |
| `WSE_LEASE_RENEW_SEC` | 5 | 만료 전 실패 여유 확보 |
| `WSE_WS_AUTH_TIMEOUT_SEC` | 5 | WSS 연결 후 ticket 인증 |

샘플은 `schemas/config.example.toml` 참조. 운영에서는 환경/Secret 관리 도구로 비밀을 주입한다.

## 3. Kubernetes 운영시 유의점

- readiness는 **Gateway와 필수 Region이 명령을 처리할 준비**가 되었을 때만 true.
- liveness 실패가 Region 저장 중에 강제 재시작을 반복하지 않도록 graceful shutdown/terminationGracePeriod 구성.
- 리더/리스 fencing은 K8s Lease만 믿지 말고 **DB 트랜잭션에서 검증**.
- CPU request/limit과 Region active NPC 수를 매칭; 스레드 수가 과도하면 CPU scheduling 지연.
- 지표 수집은 Prometheus; 로그 추적은 trace ID 포함 JSON; data stores는 백업/PITR.

## 4. 보안 위협 모델

| 위험 | 대책 |
|---|---|
| 플레이어가 actor_id 위조 | token↔character ownership 서버 검증 |
| 중복 구매/아이템 복사 | command receipt + DB transaction |
| 클라이언트 이동 속도 조작 | 서버 검증/서버 권한 위치 |
| WebSocket 대량 연결 | 인증 전 timeout, IP/session rate limit, msg max size |
| 이벤트 DSL을 통한 임의 실행 | 제한 AST + 타입 검증, OS/SQL 호출 금지 |
| LLM prompt injection | LLM은 제안만; server action whitelist/검증 |
| 오래된 Worker의 쓰기 | Epoch fencing/DB lease lock |
| 운영 API 남용 | 관리자 RBAC, MFA/감사 로그, 네트워크 제한 |
| Secret 유출 | Secret store, 로깅 마스킹, 키 회전 |
| 백업 데이터 유출 | 백업 암호화/접근 통제, 보존 정책 |

## 5. 장애 Runbook

### DB 연결 실패
1. `wse_persistence_latency_seconds`, connection health, Postgres alert 확인.
2. 신규 durable 명령 admission 중단; 보류 명령 상태는 `PENDING_COMMIT` 유지 또는 receipt resolve.
3. 데이터베이스 복구 후 lease/epoch 갱신 및 command receipt 조회.
4. 미해결 거래는 기존 command_id로 조회하며 중복 실행하지 않음.
5. backlog/Outbox와 inventory·wallet invariant 검증.

### Region Worker 종료/Deadlock
1. Region `FAILED/PAUSED`; Gateway에 `REGION_UNAVAILABLE` 노출.
2. 이전 lease 만료 또는 강제 fencing 후 새 epoch 획득.
3. 최신 유효 Snapshot 로드, durable event/reconciliation.
4. 핵심 entity count/재산/상태 hash 검증 후 ACTIVE.
5. 클라이언트에 `RESYNC_REQUIRED`를 보내 full snapshot으로 재연결.

### 이벤트 진행 중단
1. `event_id`, `correlation_id`, `tick`, `rule_id` 조회.
2. 조건 미충족인지, queue full인지, cooldown인지, rejected command인지 구분.
3. Retry 정책/timeout/fallback 적용; 동일 Event 이중 반영 금지.
4. 테스트 재현용 initial state/seed/ordered inputs 저장.

## 6. 백업과 복구

- PostgreSQL logical/physical backup 정책과 PITR(WAL)을 환경에 맞게 구성.
- 월간 복구 훈련으로 실제 RPO/RTO 측정; 초기 RPO/RTO는 **미확정**.
- snapshot과 DB event/wallet projection 사이의 consistency watermark 필수.
- Backup만 존재하고 복구 훈련이 없다면 운영 준비 완료로 처리하지 않는다.

## 7. 롤백 정책

- 코드는 Git Tag/이미지 digest로 고정.
- Schema는 Expand/Contract 방식; 이벤트 스키마 upcaster/다운그레이드 한계 문서화.
- 영속 이벤트를 기록한 새 버전에서 구 버전으로 직접 롤백할 수 있는지는 검증 필요. 불가능하면 이전 스냅샷+이벤트 변환 또는 마이그레이션 작업을 따로 준비한다.
