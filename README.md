# RPG World Simulation Engine (RPG-WSE)

**한국어** | [English](README.en.md)

> Rust 기반 실시간·지속형 RPG 시뮬레이션 엔진입니다. 현재 개발 중인 프로토타입으로, 설계 목표와 구현·통합 완료 여부를 구분합니다.
> 기술: **Rust + Bevy ECS + Tokio + Axum + PostgreSQL + Vue 3/Phaser**
> 대상: 새로운 실시간·지속형 중세 판타지 RPG 시뮬레이션. 기존 저장소와의 호환성은 가정하지 않는다.

처음 참여한다면 아래 **빠른 시작 → 저장소 구조 → 변경 검증 → 기여 방법** 순서로 읽어 주세요. [한국어 문서 목록](docs/README.md)과 [English documentation](docs/en/README.md)에서 전체 문서를 찾을 수 있습니다.

## 0. 먼저 읽기

1. [`docs/00-vision-scope.md`](docs/00-vision-scope.md) — 게임 목표, MVP, 비목표, 용어
2. [`docs/01-requirements.md`](docs/01-requirements.md) — 기능/비기능 요구사항과 추적 ID
3. [`docs/02-hld-architecture.md`](docs/02-hld-architecture.md) — 전체 아키텍처와 책임 경계
4. [`docs/03-core-tick.md`](docs/03-core-tick.md) — 시뮬레이션 코어, 고정 Tick, 결정성
5. [`docs/04-region-ecs.md`](docs/04-region-ecs.md) — ECS·Region Worker·소유권 및 이관
6. [`docs/05-command-event.md`](docs/05-command-event.md) — Command/Event, 검증, 순서, 중복 방지
7. [`docs/06-npc-ai.md`](docs/06-npc-ai.md) — Utility AI + GOAP + 기억과 LLM 경계
8. [`docs/07-world-systems.md`](docs/07-world-systems.md) — 이동, 전투, 경제, 세력, 환경, 동적 사건
9. [`docs/08-data-erd.md`](docs/08-data-erd.md) — ERD 및 영속/일시 데이터 소유권
10. [`docs/09-persistence-recovery.md`](docs/09-persistence-recovery.md) — DB 트랜잭션, Outbox, Snapshot, 복구
11. [`docs/10-network-replication.md`](docs/10-network-replication.md) — WebSocket, AOI, State Snapshot/Delta
12. [`docs/11-protocols.md`](docs/11-protocols.md) — 메시지 계약/오류 코드/관리 API 예시
13. [`docs/12-test-observability.md`](docs/12-test-observability.md) — 테스트 계획, 벤치마크, 지표
14. [`docs/13-implementation-backlog.md`](docs/13-implementation-backlog.md) — 선행관계 포함 구현 티켓·인수 기준
15. [`docs/14-operations-security.md`](docs/14-operations-security.md) — 운영·배포·보안·재해 복구
16. [`docs/15-adrs.md`](docs/15-adrs.md) — 확정 설계 결정(ADR)·미결정 목록
17. [`docs/16-prologue-gameplay.md`](docs/16-prologue-gameplay.md) — 플레이 경험, 프롤로그·공유 세계 진입
18. [`docs/17-developer-onboarding.md`](docs/17-developer-onboarding.md) — 신규 개발자 작업 절차
19. [`docs/18-end-to-end-example.md`](docs/18-end-to-end-example.md) — 구매·경제 이벤트 통합 시나리오
20. [`schemas/0001_initial.sql`](schemas/0001_initial.sql) — PostgreSQL 초기 스키마 초안
21. [`schemas/config.example.toml`](schemas/config.example.toml) — 환경 설정 예시

## 1. 이 문서를 활용하는 순서

- **기획/설계**: 00 → 01 → 02 → 15 → 16
- **엔진 개발**: 03 → 04 → 05 → 06 → 07
- **데이터/서버 개발**: 08 → 09 → 10 → 11 → SQL
- **QA/운영**: 12 → 13 → 14
- **새 개발자 온보딩**: 이 README, ADR, 담당 모듈 문서, 해당 구현 티켓의 *Definition of Done*을 순서대로 읽는다.

## 2. 실행 목표 / 개발 우선순위

**첫 Vertical Slice:** 64×64 타일 개척촌, 1명 플레이어, NPC 10명, 몬스터 2종, 10Hz 고정 Tick, 이동/대화/기본 전투, 식량 생산·소비, 상인 수송대 습격에서 시작되는 동적 사건 1개, 서버 재시작 시 복구. 이후 동시 플레이어 10명, NPC 1,000명으로 확장 검증.

**MVP 품질 기준:** (1) 헤드리스 10,000 Tick 재현, (2) 명령/이벤트 순서 일치, (3) 영속 거래 이중 반영 없음, (4) 클라이언트 2개에서 동일 상태 관측, (5) Tick p95 30ms 이하 *초기 목표* (측정 전 보장 아님).

## 3. 구현 불변식 (절대 깨면 안 됨)

- 동일 `region_id`에 **활성 쓰기 권한자 1개**. 영속 쓰기는 `ownership_epoch` fencing 검증.
- 네트워크·LLM·DB 작업은 **시뮬레이션 Tick을 블로킹하지 않는다**.
- 모든 외부 행위는 **Command**로 제출하고, 세계 상태 변경은 Region Worker/Global Owner만 수행한다.
- 금화·인벤토리·소유권과 같은 **Durable 상태는 DB Commit/ACK 이후에만 최종 성공 처리**한다.
- 위치/애니메이션 등의 **Transient 상태는 체크포인트 수준 복구**를 명시적으로 허용한다.
- 결정적 재현은 **엔진 버전 + 초기 상태 + RNG 규칙 + 적용 Tick/정렬된 입력 + 외부 결과 기록**을 전제한다.
- NPC LLM 출력은 권한 없는 제안이며 게임 규칙이 검증한다.

## 4. 문서 규칙

`REQ-*` 요구사항, `ADR-*` 아키텍처 결정, `TASK-*` 구현 작업, `TEST-*` 테스트를 ID로 참조한다. **[확정]**은 기준선, **[가정]**은 초기값, **[추후]**는 별도 결정 필요. 코드 블록은 계약/예시이며 전체 서버 구현이나 빌드 통과를 보장하지 않는다. 모든 메시지는 `schema_version`을 보유한다. 문서 간 모순이 생기면 `15-adrs.md`의 결정과 `09-persistence-recovery.md`의 원자성 규칙을 우선하고 변경 이력을 기록한다.

한국어 원본은 `README.md`와 `docs/NN-*.md`, 영어 버전은 `README.en.md`와 `docs/en/NN-*.md`입니다. 두 언어의 ID·명령·설정값·인수 기준을 함께 유지합니다. [문서 관리 안내](docs/README.md)를 참고하세요.

## 5. 현재 구현 범위와 제한

아래는 현재 소스의 진입점과 모듈 구성을 기준으로 한 안내입니다. 코드가 존재한다는 사실이 통합 검증이나 운영 준비 완료를 의미하지는 않습니다.

| 영역 | 현재 상태 |
|---|---|
| 헤드리스 Runner | seed와 Tick 수를 받아 시뮬레이션을 실행하고 상태 hash·이벤트 수·경제 요약을 JSON으로 출력 |
| 서버 | 설정 검증, HTTP health, WebSocket ticket·gateway 경로 제공 |
| 도메인·시뮬레이션 | ECS, Command/Event, 이동·전투·NPC·경제·프롤로그 코드와 테스트 포함 |
| 영속성 | PostgreSQL 거래·snapshot·복구 모듈과 DB 통합 테스트 포함. 기본 서버 진입점은 DB 연결/전체 시뮬레이션 초기화를 수행하지 않음 |
| 웹 클라이언트 | Vue 3 + TypeScript + Phaser UI와 통신 코드 포함. 로컬 연결에는 API/WS 프록시 설정 필요 |
| readiness | `/health/ready`는 현재 항상 **503**. 전체 게임의 준비 완료를 나타내지 않음 |

기본 gateway는 `GatewayState::default()`로 생성되며 시뮬레이션 명령 채널이 연결되지 않습니다. 서버와 UI를 각각 실행했다고 해서 이동·전투·영속 저장을 갖춘 전체 게임이 동작하는 것은 아닙니다. 프롤로그 파일 저장 기본 경로는 `.data/prologue`입니다.

[개발 계획](docs/19-nmain-development-plan.md), [작업 추적표](docs/20-development-status.md), [M0 ADR](docs/21-m0-bootstrap-decisions.md)은 배경 자료입니다. 일부 문서에는 과거 M0 설명과 `nmain` 브랜치 정책이 남아 있으므로 실제 코드·현재 PR 대상 브랜치와 함께 확인하세요. 현재 체크아웃에는 과거 안내의 `scripts/verify-m0.sh`, `scripts/sync-design-docs.py`, `.github/workflows/wse-m0.yml`이 없습니다. 아래 개별 명령으로 검증하세요. 현재 문서 생성에는 `scripts/sync-design-docs.mjs`를 사용합니다.

## 6. 빠른 시작

### 준비물

| 도구 | 용도 및 기준 |
|---|---|
| Git | 저장소 clone과 변경 관리 |
| Rust / Cargo | `rust-toolchain.toml`에 고정된 **1.90.0**, rustfmt·clippy 포함 |
| 네이티브 빌드 도구 | Windows MSVC는 Visual Studio Build Tools의 C++ 도구, Linux/macOS는 플랫폼에 맞는 빌드 도구 |
| Node.js / npm | 웹 개발 시 필요. 정확한 Node 버전은 저장소에 아직 고정되지 않음. 사용 버전을 PR에 기록 |
| Docker + Compose v2 | DB 통합 테스트 시 사용. Compose의 PostgreSQL 이미지는 16.11 |
| Python 3 + websockets | 기존 브라우저 E2E 스크립트를 수정·실행할 때만 필요 |

저장소를 Fork한 뒤 본인 Fork의 URL로 clone합니다. `<YOUR_FORK_CLONE_URL>`은 실제 주소로 바꾸세요. Rust 설치 후 rustup을 사용할 수 있는 터미널에서 실행합니다.

```sh
git clone <YOUR_FORK_CLONE_URL> RPG_WSE
cd RPG_WSE
rustup toolchain install 1.90.0 --profile minimal --component rustfmt --component clippy
cargo build --workspace --locked
```

아래 Rust 명령은 모두 **저장소 루트**에서 실행합니다. 설정 파일의 기본 경로도 현재 작업 디렉터리를 기준으로 해석합니다.

### 서버 실행

```sh
cargo run --locked -p game-server -- --check-config
cargo run --locked -p game-server
```

다른 터미널에서 확인합니다. Windows PowerShell에서는 `curl` 대신 `curl.exe`를 사용하세요.

```sh
curl -i http://127.0.0.1:18080/health/live
curl -i http://127.0.0.1:18080/health/ready
```

예상 응답은 live **200**, ready **503**입니다. `--check-config`는 설정만 검사하고 종료하며 DB 연결이나 게임 실행을 검증하지 않습니다. 서버는 Ctrl+C로 종료합니다.

### 헤드리스 시뮬레이션

DB와 웹 클라이언트 없이 실행할 수 있습니다.

```sh
cargo run --locked -p simulation-runner -- --seed 42 --ticks 10000
cargo run --locked -p simulation-runner -- --seed 42 --ticks 10000 --check
```

첫 명령은 실제 Tick을 실행하며 성공 시 `status: completed`, `executed_ticks`, `final_state_hash`, `economy` 등을 출력합니다. 두 번째는 인자 검증만 수행하며 `executed_ticks=0`입니다. 같은 버전·초기 상태·seed·입력 조건에서 반복 실행한 결과를 비교하세요. 최종 상태 hash 하나를 전체 경제 처리나 외부 시스템의 결정성 증거로 해석하지 마세요.

### 웹 클라이언트

별도 터미널에서 실행합니다. 루트가 아닌 `apps/web-client`의 package/lock 파일을 사용합니다.

```sh
cd apps/web-client
npm ci
npm run dev -- --host 127.0.0.1
```

`http://127.0.0.1:3000`에서 UI를 확인합니다. 포트가 사용 중이면 Vite 출력의 실제 URL을 확인하세요.

현재 클라이언트는 페이지와 같은 origin으로 `/api/v1/ws-tickets`와 `/ws`를 요청하지만 Vite 설정에는 프록시가 없습니다. 서버 연결을 개발하려면 [vite.config.ts](apps/web-client/vite.config.ts)의 기존 `server` 객체에 다음 `proxy`를 추가할 수 있습니다. 서버 포트를 바꿨다면 target도 맞추세요.

```ts
proxy: {
  '/api': { target: 'http://127.0.0.1:18080', changeOrigin: true },
  '/ws': { target: 'ws://127.0.0.1:18080', ws: true, changeOrigin: true },
},
```

변경 후 Vite를 재시작하세요. 이는 통신 경로를 연결하는 설정이며 앞서 설명한 서버의 게임 루프 통합 제한은 그대로 적용됩니다. UI의 초기 금화·인벤토리 등에는 샘플 값이 포함되어 있습니다.

## 7. 저장소 구조

| 경로 | 역할 |
|---|---|
| `apps/game-server` | 서버 진입점, TOML/환경 변수 설정 로더 |
| `apps/simulation-runner` | 헤드리스 시뮬레이션 CLI |
| `apps/web-client` | Vue/Phaser 화면, 통신 클라이언트, Vitest 테스트 |
| `crates/game-domain` | 도메인 ID, Command/Event, 게임 규칙 계약 |
| `crates/ecs-components` | ECS 컴포넌트와 Region world |
| `crates/simulation-core` | Tick 실행, 결정성, Worker 기반 |
| `crates/simulation-systems` | 이동·전투·NPC·경제·프롤로그 시스템 |
| `crates/persistence` | DB 거래, lease, snapshot, 복구와 통합 테스트 |
| `crates/networking` | Axum gateway, 세션, ticket, AOI, 상태 복제 |
| `crates/shared-protocol` | 서버/클라이언트 wire protocol |
| `schemas` | 초기 SQL과 서버 설정 예시 |
| `deploy` | 개발용 PostgreSQL Compose와 환경 변수 예시 |
| `docs` | 요구사항, 설계, ADR, 구현 백로그 |
| `tests` | 브라우저 E2E 스크립트 |

루트 `docker-compose.yml`은 이전 Go 게임용 참조입니다. 현재 체크아웃에 없는 `src/` 등을 참조하므로 신규 Rust 엔진 실행에는 사용하지 마세요.

## 8. 설정

서버는 기본적으로 [schemas/config.example.toml](schemas/config.example.toml)을 읽고 지원하는 환경 변수로 덮어씁니다. 다른 파일은 `cargo run --locked -p game-server -- --config <CONFIG_PATH>`로 지정합니다. 설정 검증 통과가 각 런타임 모듈의 설정 연결 완료를 뜻하지는 않습니다.

| 환경 변수 | 기본값 | 의미 |
|---|---|---|
| `WSE_HTTP_LISTEN` | `127.0.0.1:18080` | HTTP/WS 수신 주소 |
| `WSE_WS_AUTH_TIMEOUT_SEC` | `5` | 설정상 WS 인증 제한 시간 |
| `WSE_TICK_RATE_HZ` | `10` | Tick 빈도 |
| `WSE_GAME_TIME_SCALE` | `10` | 게임 시간 배율 |
| `WSE_REGION_CMD_CAP` | `4096` | Region 명령 큐 용량 |
| `WSE_DURABLE_CAP` | `512` | 영속 작업 큐 용량 |
| `WSE_SNAPSHOT_INTERVAL_SEC` | `300` | snapshot 간격 |
| `WSE_REGION_LEASE_SEC` | `15` | Region lease 기간 |
| `WSE_LEASE_RENEW_SEC` | `5` | lease 갱신 간격 |
| `RUST_LOG` | `info` | tracing 로그 필터 |
| `PROLOGUE_DATA_DIR` | `.data/prologue` | gateway의 프롤로그 파일 저장 경로 |
| `DATABASE_URL` | 명시적으로 지정 | PostgreSQL 통합 테스트 연결 문자열 |

예를 들어 서버 포트를 바꾸려면:

```sh
# Bash / zsh
WSE_HTTP_LISTEN=127.0.0.1:19080 cargo run --locked -p game-server
```

```powershell
# PowerShell: 현재 터미널에 적용
$env:WSE_HTTP_LISTEN = '127.0.0.1:19080'
cargo run --locked -p game-server
Remove-Item Env:WSE_HTTP_LISTEN
```

`deploy/.env`는 Docker Compose용입니다. Rust 서버가 이 파일을 자동으로 읽지는 않습니다. Tick 빈도는 1000ms를 나누어떨어지게 해야 하고 delta 빈도보다 작을 수 없습니다. lease 갱신 간격은 lease 기간보다 짧아야 합니다.

## 9. 개발용 PostgreSQL

영속성 작업에만 필요한 선택 단계입니다. 테스트 데이터를 기록하는 **전용 개발 DB**를 사용하세요.

```sh
# Bash / zsh: 최초 1회만 복사
cp deploy/.env.example deploy/.env
```

```powershell
# PowerShell: 최초 1회만 복사
Copy-Item deploy/.env.example deploy/.env
```

`deploy/.env`의 `WSE_POSTGRES_PASSWORD`를 본인 개발용 값으로 바꾼 뒤 실행합니다.

```sh
docker compose --env-file deploy/.env -f deploy/compose.dev.yml up -d --wait
docker compose --env-file deploy/.env -f deploy/compose.dev.yml ps
```

기본 접속은 `127.0.0.1:55432`, 사용자/DB는 `wse_dev`입니다. 테스트를 실행할 터미널에 연결 문자열을 설정하세요. 비밀번호 자리표시자를 실제 값으로 바꾸고 URI 특수문자는 percent-encoding하세요.

```sh
export DATABASE_URL='postgres://wse_dev:<YOUR_DEV_PASSWORD>@127.0.0.1:55432/wse_dev'
```

```powershell
$env:DATABASE_URL = 'postgres://wse_dev:<YOUR_DEV_PASSWORD>@127.0.0.1:55432/wse_dev'
```

Compose는 SQL을 자동 적용하지 않습니다. 영속성 통합 테스트는 접속 후 `apply_migration`으로 초기 스키마를 적용합니다. 기존 DB 업그레이드 절차와 동일하게 취급하지 마세요.

```sh
cargo test --locked -p persistence --test durable_transactions -- --nocapture
cargo test --locked -p persistence --test snapshot_recovery -- --nocapture
# 종료: 볼륨 데이터는 유지
docker compose --env-file deploy/.env -f deploy/compose.dev.yml down
```

**DB에 접속하지 못하면 현재 테스트는 실패 대신 skip 후 성공으로 보일 수 있습니다.** 출력에 `Skipping PostgreSQL integration test`가 없는지 확인하고 실제 DB 검증 여부를 PR에 별도로 기록하세요. 비밀번호를 바꿔도 기존 PostgreSQL 볼륨의 비밀번호는 자동 변경되지 않습니다.

## 10. 변경 검증

Rust 변경은 저장소 루트에서 실행합니다.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

웹 변경은 `apps/web-client`에서 실행합니다.

```sh
npm ci
npm test
npm run typecheck
npm run build
```

필요한 모듈을 먼저 확인할 때는 `cargo test --locked -p simulation-core`처럼 패키지를 좁힐 수 있습니다. 결과를 보고할 때는 DB skip, 실행하지 않은 브라우저 검증, 실패 항목도 구분하세요. 위 명령은 검증 절차이며 현재 체크아웃의 통과 보증은 아닙니다.

[브라우저 E2E](tests/e2e_two_clients_prologue.py)는 스크립트 위치를 기준으로 저장소 경로를 찾으며, 브라우저는 `CHROME_BIN` 또는 PATH에서 찾습니다. Chrome CDP 포트 9222와 preview 포트 4173을 사용하므로 브라우저 실행 옵션·API/WS 라우팅을 환경에 맞게 정리한 뒤 사용해야 합니다. 일반 Rust/Vitest 실행에 Python은 필요하지 않습니다.

### 자주 겪는 문제

| 증상 | 확인할 내용 |
|---|---|
| 설정 파일을 못 찾음 | 저장소 루트에서 실행했는지, `--config` 경로가 맞는지 확인 |
| ready가 503 | 현재 서버의 의도된 응답. DB만 켠다고 200이 되지 않음 |
| 서버 bind 실패 | 18080 포트 사용 여부 확인, `WSE_HTTP_LISTEN` 변경 |
| 화면은 뜨지만 ticket 요청이 404 | Vite `/api` 프록시 및 Rust 서버 실행 여부 확인 |
| WS 연결 실패 | `/ws` 프록시의 `ws: true`, target 포트, 브라우저 Network/WS 확인 |
| DB 테스트가 너무 빨리 성공 | `--nocapture`로 skip 메시지와 `DATABASE_URL` 확인 |
| PowerShell에서 npm 스크립트 실행 차단 | `npm.cmd ci`, `npm.cmd test`처럼 실행 파일을 직접 사용 |
| `--locked` 오류 | 의존성 변경이 의도된 것인지 확인하고 해당 lock 파일을 함께 갱신 |

## 11. 기여 방법

버그 수정, 테스트 보강, 문서 수정, 엔진과 클라이언트 개발 모두 기여할 수 있습니다. 첫 작업으로는 실행 안내 재현, DB 테스트 skip 개선, 개발 프록시 설정, E2E 경로 이식성 개선처럼 범위가 명확한 항목이 좋습니다. 큰 구조 변경은 먼저 Issue에서 요구사항과 접근 방법을 논의하세요.

1. 기존 Issue/PR과 [구현 백로그](docs/13-implementation-backlog.md)를 확인해 중복 작업을 피합니다.
2. 현재 저장소의 기본 브랜치 또는 유지관리자가 지정한 브랜치에서 작업 브랜치를 만듭니다. 과거 문서만 보고 `nmain`이 존재한다고 가정하지 마세요.
3. 변경을 한 가지 문제로 좁히고 해당 동작을 검증하는 테스트와 관련 문서를 함께 수정합니다.
4. 위 검증 명령을 실행하고 Fork에서 PR을 제출합니다. 관련 `REQ-*`, `TASK-*`, `TEST-*`가 있으면 연결합니다.

```sh
git switch -c fix/describe-the-change
# 수정과 검증 후 관련 파일만 선택해 커밋
git add <CHANGED_FILES>
git commit -m "Describe the change"
git push -u origin fix/describe-the-change
```

PR 본문에는 다음 정보를 포함해 주세요.

- **문제와 변경 결과:** 어떤 상황에서 무엇이 달라지는가
- **관련 계약:** API/DB/Command/Event와 schema_version 영향, 필요한 ADR 변경
- **검증:** 실행한 명령, 결과, OS·도구 버전, 실제 PostgreSQL 사용 여부
- **제한:** 미실행 테스트, 알려진 문제, 호환성·복구 영향
- **UI 변경:** 변경 전후 화면과 브라우저 확인 결과

Rust는 rustfmt·clippy 기준을 따릅니다. Cargo.lock과 웹 package-lock.json은 의존성 변경 시 함께 관리하고 단순 코드 변경에 불필요한 lock 갱신을 섞지 마세요. 공유 프로토콜 변경은 Rust 계약과 TypeScript 클라이언트 양쪽을 확인하세요. 아키텍처 변경은 [ADR](docs/15-adrs.md)에 이유와 영향을 기록하고 3절의 단일 Writer·비차단 Tick·Commit/ACK 불변식을 지킵니다.

버그 Issue에는 재현 단계, 기대/실제 결과, 커밋, OS·도구 버전, 민감정보를 제거한 로그를 첨부하세요. 결정성 문제에는 seed·Tick 수·입력 조건도 필요합니다. 비밀번호, ticket, 실제 계정 데이터, 개인 접속 문자열은 커밋이나 공개 Issue에 포함하지 마세요. 보안 취약점은 저장소의 비공개 신고 기능이 제공되는 경우 해당 경로를 사용하세요.

## 12. 라이선스

프로젝트 루트의 [LICENSE](LICENSE)는 **Apache License 2.0**입니다. 외부 라이브러리와 에셋에는 각자의 라이선스가 적용됩니다. 에셋을 추가할 때는 출처·라이선스·재배포 조건을 함께 기록해 주세요. [클라이언트 README](apps/web-client/README.md)의 에셋 설명은 프로젝트 전체 라이선스를 대체하지 않습니다.
