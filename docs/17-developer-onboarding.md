# 17. 신규 개발자 온보딩 · 작업 시작 가이드

**한국어** | [English](en/17-developer-onboarding.md)

## 1. 이 문서로 합류한 개발자의 첫 작업

### Step 1. 게임 의도·범위 확인

`README → 00-vision-scope → 01-requirements → 02-hld-architecture → 15-adrs` 순서로 읽는다. 본인이 맡을 작업에 관련된 문서는 추가로 읽는다. 예를 들어 Tick 담당자는 03·04·05·12를 읽는다.

### Step 2. 최소 개발 환경 준비

- Rust stable + Cargo (버전은 팀에서 `rust-toolchain.toml`로 고정)
- PostgreSQL 16 이상 테스트 인스턴스(영속 모듈 담당 시)
- `cargo fmt`, `cargo clippy`, `cargo test`
- Docker 또는 컨테이너 실행 환경(통합 테스트 시)
- Git, DB migration tool (`sqlx-cli` 등), Grafana/Prometheus(성능 담당 시)

```bash
# 프로젝트 저장소를 팀이 준비한 뒤 실행할 명령의 기준
rustc --version
cargo --version
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**설계 작성 당시 안내:** 최초 패키지는 Cargo 구현 전의 설계 문서 세트였으며 TASK-001에서 Workspace를 만들고 Rust/SQLx 버전을 고정하도록 계획했습니다. 현재 체크아웃에는 Workspace가 있으므로 실제 실행 방법과 제한은 [루트 README](../README.md)를 따르세요.

### Step 3. 첫 작업 선정

- 엔진 담당: `TASK-001 → TASK-002 → TASK-003`
- 도메인 담당: `TASK-004 → TASK-005 → TASK-010/011`
- DB 담당: `TASK-007 → TASK-008`
- 서버/클라이언트 담당: `TASK-006 → TASK-009 → TASK-012`
- QA/운영 담당: `TASK-013 → TASK-014`

**병렬 작업 조건:** API/이벤트/DB 계약이 먼저 리뷰되어야 후속 모듈을 독립 개발할 수 있다. 공유 계약 변경은 해당 인터페이스 소유자에게 리뷰 요청.

## 2. 작업 시작 전 반드시 답할 질문

1. 담당 상태의 **진실 원천**이 ECS인지 PostgreSQL인지?
2. 상태를 바꾸는 **유일한 Writer**는 누구인지?
3. 같은 Command가 100회 반복되면 결과가 어떻게 되는지?
4. 서버가 DB Commit 직후 죽으면 어떻게 복구되는지?
5. 이 작업은 몇 Tick에 적용되며 어떤 순서로 Replay되는지?
6. 성공/실패를 어떤 이벤트·메트릭·테스트로 관측하는지?

하나라도 불명확하면 PR보다 먼저 ADR/설계 명세를 보강한다.

## 3. 디렉터리와 소유자

| 디렉터리 | 책임 | 리뷰 대상 |
|---|---|---|
| `crates/simulation-core` | Tick, Region, ownership | Core/Architect |
| `crates/ecs-components` | Component contracts, Stable ID | Core/Domain |
| `crates/simulation-systems` | Movement/Combat/NPC/Economy | Domain/Core |
| `crates/game-domain` | Command/Event/rules | Architect/Domain |
| `crates/persistence` | DB transaction/snapshot/recovery | DB/Core |
| `crates/networking` | Axum/session/AOI/delta | Backend/Security |
| `crates/shared-protocol` | 외부 메시지 버전 | Backend/Frontend |
| `apps/simulation-runner` | 결정적 재현/부하 | QA/Core |

## 4. 구현 과정 예시: 새 NPC 액션 추가

1. 요구사항 ID(예: `REQ-F-006`)와 수행 TEST ID 확인.
2. AI Action 카탈로그에 `action_id`, preconditions, effects, cost/timeout 정의.
3. GOAP는 Action 실행을 명령으로 예약할 뿐 상태를 직접 변경하지 않도록 구현.
4. 다음 Tick에서 Command Handler가 거리/자원/상태 재검증.
5. durable 결과라면 DB Writer의 receipt/transaction/ACK를 경유.
6. Domain Event가 NPC 기억/퀘스트에 영향을 주는 경우 규칙을 추가.
7. Golden replay, 실패/중복 테스트 통과 및 이벤트 추적 로그 확인.

## 5. 문제 발생 시 1차 진단

| 현상 | 우선 확인 |
|---|---|
| 자동이동 클릭 무반응 | Client COMMAND 발생 여부→Gateway receipt→Region queue→Navigation state→Reject reason |
| NPC 정지 | next_decision_tick, Utility score, GOAP fail reason, budget exhaustion |
| 퀘스트 다음 단계 미진행 | rule prerequisite, event chain causation, cooldown, tick, outbox/retry |
| 아이템 중복 | command_id, receipt hash, DB tx/unique, actor reservation |
| 재접속 상태 불일치 | snapshot baseline, delta seq gap, Region revision, reconnect path |
| Worker 재기동 무한 루프 | lease epoch, checkpoint checksum, migration/upcaster, durable reconciliation |
| 성능 저하 | Tick per-system duration, path expansion, AI evaluations, channel depth |

## 6. 리뷰 / 인계 형식

각 TASK를 끝낼 때 다음을 남긴다.

```markdown
### Task completion: TASK-XXX
- 구현한 코드 위치:
- 문서/ADR 변경:
- API/DB/이벤트 schema_version:
- 통과한 테스트와 커맨드:
- 측정한 성능과 환경:
- 실패 복구/rollback 절차:
- 알려진 제한과 후속 TASK:
```
