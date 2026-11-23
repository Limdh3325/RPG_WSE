# 21. 운영 런북 및 배포·복구 매뉴얼 (Operations Runbook)

**한국어** | [English](en/21-operations-runbook.md)

기준: nmain WSE 신규 아키텍처 (M8 / TASK-014 배포·복구 훈련 완료 기준).

---

## 1. 시스템 아키텍처 및 엔드포인트 구성

WSE(World Simulation Engine)는 권한 있는 결정론적 게임 루프, 비동기 WebSocket 게이트웨이, PostgreSQL 영속성 계층, 그리고 Vue.js 웹 클라이언트로 구성됩니다.

### 엔드포인트 맵

| 경로 | 메서드 | 용도 | 응답 코드 및 설명 |
|---|---|---|---|
| `/health/live` | GET | Liveness Probe (컨테이너 생존 검사) | `200 OK` (문자열 `"OK"`) |
| `/health/ready` | GET | Readiness Probe (트래픽 수신 가능 여부) | `200 OK` (`"READY"`) 또는 `503 Service Unavailable` (`"DRAINING"`) |
| `/api/v1/admin/drain` | POST | Graceful Drain 시작 (관리자 전용) | Bearer 토큰 검증, `200 OK` (드레인 시작) / `401 Unauthorized` / `403 Forbidden` |
| `/api/v1/ws-tickets` | POST | WebSocket 1회성 접속 티켓 발급 | 드레인 중에는 `503 Service Unavailable` 반환 |
| `/api/v1/regions/:id/status` | GET | 특정 리전의 Epoch, Seq, 활성 구독자, 엔티티 상태 | JSON 응답 |
| `/metrics` | GET | Prometheus Scrape 엔드포인트 | `text/plain; version=0.0.4`, 12종 핵심 메트릭 |
| `/ws` | GET (Upgrade) | 양방향 WebSocket 통신 게이트웨이 | 상태 복제, AOI 델타, 클라이언트 커맨드 처리 |

---

## 2. 환경 변수 및 시크릿(Secret) 주입

| 환경변수명 | 필수 여부 | 기본값 | 설명 |
|---|---|---|---|
| `DATABASE_URL` | 운영 필수 | `[REDACTED:connection_string]postgres:5432/wse_prod` | PostgreSQL 16+ 연결 문자열 |
| `WSE_ADMIN_TOKEN` | 운영 필수 | - | 관리자 RBAC 엔드포인트 (`/api/v1/admin/drain`) 인증 Bearer 토큰 |
| `WSE_POSTGRES_PASSWORD` | 필수 | - | DB 사용자 비밀번호 |
| `PROLOGUE_DATA_DIR` | 선택 | `/app/data/prologue` | 개인 프롤로그 세션 영속 디렉토리 |
| `RUST_LOG` | 선택 | `info` | 로그 레벨 (`info`, `debug`, `trace` 등) |

> **보안 주의**: 일체의 비밀번호나 API 시크릿, 토큰을 Git 저장소에 커밋하지 마십시오. 배포 시 `deploy/.env` 또는 오케스트레이터의 Secret Store를 통해 주입합니다.

---

## 3. 프로덕션 배포 및 롤아웃 절차

### 3.1 컨테이너 기동
```bash
# 1. 템플릿 복사 및 운영 시크릿 설정
cp deploy/.env.prod.example deploy/.env
# deploy/.env 파일 내 WSE_POSTGRES_PASSWORD 및 WSE_ADMIN_TOKEN 설정

# 2. 프로덕션 Compose 빌드 및 백그라운드 기동
docker compose --env-file deploy/.env -f deploy/compose.prod.yml up -d --build --wait

# 3. 서비스 상태 및 헬스체크 확인
docker compose --env-file deploy/.env -f deploy/compose.prod.yml ps
```

---

## 4. 무중단 롤링 업데이트 및 Graceful Drain 런북

신규 버전 배포 또는 서버 점검 시 세션 단절 및 상태 손실을 방지하기 위해 Graceful Drain 절차를 수행합니다.

### 4.1 Drain 절차
1. **관리자 Drain 명령 전송**:
   ```bash
   curl -X POST http://127.0.0.1:18080/api/v1/admin/drain \
     -H "Authorization: Bearer $WSE_ADMIN_TOKEN" \
     -H "Content-Type: application/json"
   ```
2. **트래픽 차단 검증**:
   - `/health/ready` 엔드포인트가 즉시 `503 Service Unavailable ("DRAINING")`을 반환하여 인그레스/로드밸런서가 신규 트래픽을 차단합니다.
   - 신규 WebSocket 티켓 발급(`/api/v1/ws-tickets`)이 즉시 503으로 거절됩니다.
3. **세션 드레인 및 상태 플러시 대기**:
   - 기존 연결된 세션의 인플라이트 커맨드가 완료되고, 미저장 스냅샷 및 프롤로그 진행도가 영속 저장소에 완전히 동기화됩니다 (권장 대기 시간: 10~15초).
4. **신규 컨테이너 교체 및 이전 컨테이너 종료**:
   ```bash
   docker compose --env-file deploy/.env -f deploy/compose.prod.yml up -d --no-deps --build game-server
   ```

---

## 5. 장애 대응 런북 (Incident Scenarios)

### 시나리오 A: PostgreSQL 일시 단절 및 트랜잭션 롤백
- **증상**: 영속 커밋 지연 및 회로 차단기(Circuit Breaker) 발동.
- **동작**: WSE는 인메모리 시뮬레이션을 중단하지 않고, 영속 거래 요청에 대해 멱등한 Retryable 에러를 반환하며 이전 상태를 안전하게 격리합니다.
- **조치**:
  1. PostgreSQL 헬스체크 확인: `pg_isready -h 127.0.0.1 -p 5432`.
  2. 연결 복구 시 미처리 인텐트(Outbox/Pending)가 원자적으로 재시도 및 반영됨을 로그로 확인.

### 시나리오 B: Gateway/Worker 프로세스 크래시
- **증상**: 컨테이너 비정상 종료 (SIGKILL 등).
- **복구 목표**: RPO = 0 (트랜잭션 데이터 무손실), RTO < 5s (로컬 재기동 기준).
- **조치**:
  1. `FilePrologueStorage` 및 PostgreSQL WAL 기반 복구 기동.
  2. 재기동 완료 후 `/health/ready`가 `200 OK`로 복귀함을 확인.
  3. 클라이언트는 지수 백오프로 재접속하며, 게이트웨이는 `WorldSnapshot` 재동기화 메시지를 통해 즉시 최신 상태를 동기화.

### 시나리오 C: Slow Client 또는 네트워크 지연으로 인한 큐 적체
- **동작**: 개별 느린 클라이언트는 백프레셔에 의해 버퍼 용량(256건) 초과 시 해당 세션만 정상 분리(disconnect)되며, 시뮬레이션 메인 루프나 타 플레이어 틱에 절대 영향을 주지 않습니다 (`TEST-NET-006` 검증 완료).

### 시나리오 D: 비정상 명령 폭주 및 위조 공격
- **동작**: 세션당 초당 60회를 초과하는 명령은 `Rate limit exceeded`로 거절되고, 위조된 `actor_id`는 즉시 `auth_mismatch`로 거절되며 `wse_command_rejections_total` 메트릭에 기록됩니다.

---

## 6. 백업 및 재해 복구(DR) 드릴

### 6.1 정기 백업 및 PITR 훈련 실행
```bash
# 백업 및 RTO 실측 스크립트 실행 (RTO <= 60s SLA 검증)
./scripts/backup-restore-drill.sh
```

### 6.2 롤백 체크리스트
- [ ] 신규 버전 배포 전 직전 릴리스 이미지 태그/다이제스트 보존 여부 확인.
- [ ] DB 마이그레이션이 하위 호환(Backward compatible)인지 확인.
- [ ] 롤백 필요 시 Graceful Drain 실행 후 이전 버전 이미지로 재배포:
  ```bash
  docker compose --env-file deploy/.env -f deploy/compose.prod.yml rollback
  ```
