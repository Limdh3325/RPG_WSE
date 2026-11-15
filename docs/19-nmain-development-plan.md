# nmain 신규 개발 상세 실행 계획

**한국어** | [English](en/19-nmain-development-plan.md)

작성일: 2026-11-09 (Asia/Seoul)
상태: 구현 착수용 계획. 최신 진행 상태는 20-development-status.md와 outbox 완료 보고서를 참조한다.

구현 대상은 개발자가 준비한 환경의 저장소 체크아웃이다. 서버 주소와 설치 경로는 각 환경에 맞게 지정한다. 아래 출발점과 nmain 브랜치 명칭은 계획 작성 당시의 기록이다.

## 1. 목표와 현재 출발점

README와 docs/00~18의 설계 기준선을 기반으로 Rust + Bevy ECS + Tokio + Axum + PostgreSQL + Vue 3/Phaser 게임을 신규 개발한다. 기존 Go/React 게임과의 API·DB·저장 데이터 호환은 첫 릴리스 범위에 포함하지 않는다. 기존 기능의 자동 이식도 가정하지 않는다.

확인한 저장소 상태:

- 작업 브랜치: nmain, 로컬 추적 대상 origin/nmain. 원격 최신 상태를 fetch한 것은 아니다.
- 시작 커밋: e75fa61 `feat:init WSE`.
- main은 3a715bc에 있으며 이 계획 작업에서 변경하지 않는다.
- 루트 README, docs/00~18, schemas/config.example.toml이 설계 기준이다.
- Cargo.toml과 apps 디렉터리는 아직 없다.
- schemas/0001_initial.sql은 로컬에 있지만 `*.sql` ignore 규칙으로 Git 추적에서 빠져 있다. 신규 체크아웃에서도 받을 수 있도록 TASK-000에서 해결한다.
- Resume.md와 Resume_KR.md는 사용자 미추적 파일이므로 개발 변경에 포함하지 않는다.

첫 목표는 64×64 개척촌에서 플레이어 1명, NPC 10명, 몬스터 2종이 움직이고, 식량 부족과 수송대 습격에 개입하며, 서버 재시작 후 확정 재산이 유지되는 작은 완성 게임이다. 2개 클라이언트의 동일 상태 관찰을 검증한 뒤 동시 플레이어 10명/NPC 1,000명으로 확장한다.

## 2. 범위와 구현 원칙

- 단일 프로세스·단일 공유 Region부터 시작한다. 프롤로그 단계에서 개인 Region과 공유 Region 간 이관을 추가한다.
- Region의 ECS 쓰기는 한 Worker만 수행한다. 네트워크·DB·LLM은 Tick 밖에서 실행한다.
- Command(의도), Domain Event(확정 사실), State Delta(표현)를 별도 타입으로 관리한다.
- 금화·인벤토리·소유권·중요 진행 상태는 DB Commit 확인 뒤 확정한다. ACK 유실은 실패로 단정하지 않고 receipt로 조회한다.
- 위치 등 transient 상태의 손실 허용 범위를 체크포인트 정책으로 명시한다.
- 초기 ECS 실행은 단일 스레드 순서를 고정한다. Region 간 실행 분리와 ECS 내부 병렬화를 혼동하지 않는다.
- 템플릿 대화와 규칙 기반 NPC로 첫 게임을 완성한다. LLM은 후속 선택 기능이다.
- 분산 샤딩, 다중 서버 이관, 지역 간 즉시 거래, 고급 PvP 보정, 대규모 월드, 기존 던전·패시브·만신전 이식은 후속 백로그다.
- Prometheus 지표와 구조화 로그는 초기부터 추가하고 최종 단계에서 대시보드·부하 검증을 완성한다.

## 3. 예정 디렉터리와 책임

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
apps/
  game-server/          # 프로세스 조립, 설정, 런타임, 종료
  simulation-runner/    # seed/입력 로그/재생/해시/벤치마크
  web-client/           # Vue 3 + TypeScript + Phaser
crates/
  game-domain/         # ID, Command/Event, 거래·오류 타입
  ecs-components/      # ECS 컴포넌트, StableId 매핑
  simulation-core/     # Tick, Region 실행, RNG, 결정적 스케줄
  simulation-systems/  # 이동, 전투, NPC, 경제, 사건 규칙
  persistence/         # SQLx, receipt, lease, outbox, snapshot
  networking/          # Axum, 인증, WS, AOI, replication
  shared-protocol/     # 버전별 wire DTO와 계약 fixture
schemas/               # SQL 마이그레이션 및 설정 예제
fixtures/              # 월드, 입력 로그, 프로토콜, golden 기준
deploy/                # 신규 개발/검증용 Compose, 모니터링
docs/                  # 설계, ADR, 실행 계획, runbook
```

도메인 규칙은 Axum·SQLx에 의존하지 않는다. persistence/networking이 ECS를 직접 수정하지 않도록 메시지 경계를 둔다. Rust/TypeScript 간 계약은 공통 fixture로 검증하고 생성 도구 채택 여부는 TASK-004에서 결정한다. 위 경로는 예정 구조이며 실제 생성은 TASK-001부터 진행한다.

## 4. 단계별 구현 티켓과 완료 기준

### M0 — 재현 가능한 개발 환경 (TASK-000~001)

**TASK-000: 저장소·설계 기준 정리 — 선행 없음**

1. 추적 파일과 남은 디렉터리를 조사하고 신규 빌드 진입점/레거시 참조를 분리한다. 일괄 삭제하지 않는다.
2. `.gitignore`에 Rust 산출물 규칙과 `schemas/*.sql` 예외를 추가하고 초기 SQL을 추적한다. 설정 예제와 필요한 마이그레이션이 새 체크아웃에 포함되는지 확인한다.
3. README의 링크·실행 안내·문서 우선순위를 확인한다. 분할 설계 문서를 편집 기준으로 삼고 통합본/manifest 재생성 방식을 정해 문서 간 드리프트를 막는다.
4. 신규 DB 이름·볼륨·포트를 분리하는 개발 Compose 계획을 확정한다. 기존 서비스·DB를 연결하지 않는다.
5. REQ→TASK→TEST 대응표와 진행 상태표를 만든다.

완료: 새 체크아웃에 설계·SQL이 모두 존재하고, 새 게임의 실행 경로가 명확하다.

**TASK-001: Rust 워크스페이스·CI — 선행 TASK-000**

1. 예정 apps/crates를 만들고 Rust edition 2024, 컴파일러, 의존성 버전을 실제 호환 확인 후 고정한다.
2. Cargo.lock, 설정 로더, 환경 변수 덮어쓰기, tracing, 오류 타입을 구성한다.
3. game-server의 health/liveness와 simulation-runner의 CLI를 만든다. readiness는 실제 준비 상태와 구분한다.
4. CI에 fmt, clippy, test를 연결한다. PostgreSQL 통합 테스트 작업은 TASK-007에서 추가한다.
5. 신규 개발용 PostgreSQL Compose와 `.env.example`을 작성한다.

완료: 빈 서버와 Runner가 실행되고 깨끗한 체크아웃에서 빌드·검사가 재현된다.

### M1 — 결정적인 헤드리스 월드 (TASK-002~004)

**TASK-002: Region·Entity 모델 — 선행 TASK-001**

- World/Region/Stable UUID와 ECS Entity 매핑, 생성·삭제·조회 및 생명주기 구현.
- Position/Velocity/Health/Needs/Inventory projection 등 최소 컴포넌트와 64×64 테스트 월드 구성.
- canonical 상태 DTO를 ECS 내부 핸들과 분리. 복원 후에도 외부 ID가 유지되도록 설계.
- 검증: 10,000 Entity 생성/삭제/재생성에서 매핑 정합성, 중복 ID 거부.

**TASK-003: Fixed Tick·RNG·재현성 — 선행 TASK-002**

- 100ms 논리 Tick과 게임 시간 배율 분리, phase 실행 순서 고정.
- RNG 알고리즘/버전과 스트림 분리 규칙 확정. UUID 생성 및 외부 결과도 재현 가능한 입력 또는 결정적 생성 규칙으로 관리.
- 안정 정렬, 정수/fixed-point 계산, canonical 직렬화 및 상태/이벤트 해시 구현.
- Runner에 seed, ticks, 입력 fixture, report, 최초 불일치 Tick 출력 추가.
- 검증: 동일 seed/초기 상태/입력/버전으로 10,000 Tick 반복 시 상태와 이벤트 순서 일치. headless는 벽시계 대기 없이 실행.

**TASK-004: Command/Event 계약 — 선행 TASK-003**

- command_id, actor/region, payload_hash, assigned_tick, ingress_seq 및 검증 오류 정의.
- Command 수신/거부/실행/저장 대기/성공/실패 상태 전이와 재조회 계약 통일.
- event_id, causation_id, correlation_id, schema/rules version, region_event_seq 정의.
- transient 중복 제거와 durable DB receipt의 보장 범위 분리. payload가 다른 command_id 재사용 거부.
- 이벤트 예산·인과 깊이·후속 Tick 예약 규칙으로 무한 연쇄 차단.
- 검증: 순서 역전·중복 입력, 규칙 순환, 알 수 없는 버전/필드 처리.

**M1 게이트:** UI·DB 없이 10,000 Tick 재현 테스트 통과. 이 단계의 메모리 중복 방지를 재시작 내구성으로 표현하지 않는다.

### M2 — 이동·전투·Worker 실행 경계 (TASK-005~006)

**TASK-005: 최소 이동·전투 — 선행 TASK-004**

- 정수 millitile 이동, 타일 충돌, A* 탐색 예산, 동적 장애물 재탐색 구현.
- 이동 수신→경로 탐색→진행→완료/실패 상태와 사유를 외부에 노출.
- 몬스터 2종, 사거리·쿨다운·피해·사망 규칙 구현.
- 전리품과 사망의 영속 효과는 durable intent로 출력하며 TASK-007 이전에는 운영 수준 지급 완료로 취급하지 않는다.
- 검증: 벽 통과/속도 조작/거리 밖 공격 거부, 길 없음·대상 사망·중복 보상 요청 처리.

**TASK-006: Region Worker·Router — 선행 TASK-004, 통합 검증 시 TASK-005**

- Region 전용 실행 스레드와 Tokio I/O 사이 bounded channel 구성.
- Command/ACK/Durable/Transient 큐를 분리하고 ACK 우선 처리·포화 정책 구현.
- Region lifecycle, single writer, epoch 검증 훅, pause/resume, 정상 종료 drain 구현.
- 두 Region 격리 테스트를 추가하되 분산 서버는 도입하지 않는다.
- 검증: 큐 포화 시 명시적 거부, 저장 지연 중 무관 Entity Tick 진행, Worker 하나의 장애가 다른 Region에 미치는 영향 확인.

### M3 — 재산 확정과 재시작 복구 (TASK-007~008)

**TASK-007: PostgreSQL 거래·Lease·Outbox — 선행 TASK-004/006**

- 초기 SQL을 일회성 빈 PostgreSQL 16+ DB에서 검증하고 버전별 migration 실행·기동 정책 구현.
- wallet/stock/inventory와 receipt/event/outbox를 하나의 트랜잭션으로 확정.
- 자산 예약, 고정 잠금 순서, version 재검증, lease/epoch fencing, ACK 후 ECS 확정 구현.
- ACK timeout은 receipt 조회로 해결. 동일 명령 재실행과 다른 payload 재사용 차단.
- Outbox 재발행과 소비자 중복 제거 구현. DB 장애 시 durable admission을 중단하고 상태를 노출.
- 검증: 재고 1개에 구매자 2명, 동일 구매 100회, Commit 전 중단, Commit 후 ACK 유실, 오래된 Worker 쓰기, Outbox 중복 발행.

**TASK-008: Checkpoint·Recovery — 선행 TASK-007**

- 신규 durable admission 중지→pending 해소→Tick 경계 snapshot 순서 구현.
- 엔진/스키마/규칙 버전, checksum, 확정 이벤트 watermark, RNG, 예약 행동을 저장.
- 새 epoch 획득→유효 snapshot 복원→DB 확정 데이터 대조→불변식 검사→ACTIVE 기동 경로 구현.
- 손상 snapshot의 이전 버전 fallback과 호환 불가 시 명시적 실패 처리.
- 검증: snapshot 중 종료, checksum 손상, Commit 이후 재시작, 타이머/RNG 복원, receipt로 미확정 결과 해결.

**M3 게이트:** 실제 PostgreSQL과 프로세스 중단 테스트에서 확정 자산 유실·중복 0. 직렬화 함수 단위 테스트만으로 복구 완료 처리하지 않는다.

### M4 — 두 클라이언트가 같은 세계 관찰 (TASK-009, TASK-012A)

**TASK-009: 인증·WebSocket·AOI — 선행 TASK-006/007, 재시작 검증 TASK-008**

- 계정/캐릭터 소유권, 세션, 짧은 수명의 1회용 WS ticket과 인증 timeout 구현.
- protocol/schema version, 메시지 크기·빈도 제한, 잘못된 입력 거부.
- AOI 셀 구독, baseline snapshot, delta_seq, ACK, gap/resync 및 느린 연결 격리 구현.
- reconnect 시 최신 상태와 pending command 결과 재조회.
- 검증: actor 위조, ticket 재사용, 두 클라이언트 상태 일치, 메시지 누락/중복/단절, queue backlog.

**TASK-012A: 최초 플레이 화면 — 선행 TASK-005/009**

- Vue 3/TypeScript/Phaser 앱, 타일 월드·캐릭터·카메라·직접 이동 구현.
- HP, 대화, 인벤토리, 명령 진행/실패 UI와 확정 후 재산 표시 구현.
- placeholder 아트로 플레이를 먼저 검증. 리소스 출처·사용 조건 기록.
- 단위/프로토콜 테스트와 실제 브라우저 2개 접속 E2E 실행.

**M4 게이트:** 로그인→접속→이동→전투→재접속을 실제 브라우저에서 수행. 프론트 빌드 성공을 플레이 검증으로 대체하지 않는다.

### M5 — NPC 생활과 사건이 이어지는 첫 게임 (TASK-010~011)

**TASK-010: Utility AI·GOAP — 선행 TASK-002/003/005**

- Perception/Needs/관계·기억, 목표 점수, 제한된 GOAP 액션과 계획 예산 구현.
- 농부 생산, 상인 교역, 경비 순찰, 도적 습격·도주 행동 구성.
- 판단을 Tick에 분산하고 다음 Tick Command로 제출. 경로 실패·대상 소멸에 fallback.
- 초기 대화는 템플릿으로 제공. 구조화된 중요 기억은 TASK-007 저장 경로에 연결.
- 검증: NPC 10명 무입력 생활, 위험 회피, 막힌 계획 해제, 잘못된 행동도 공통 검증에서 거부.

**TASK-011: 식량·시장·수송대·사건 — 선행 TASK-007/010**

- 생산/소비/재고/가격·치안·평판 규칙, 장부와 음수·중복 반영 불변식 구현.
- 임계값 false→true 전이로 부족 사건 발생, 지속 부족 상태에서 중복 생성 차단.
- 부족→보급 요청→수송대 이동→도적 습격→호위/공격/방관→결과 반영 연결.
- 제한된 규칙 표현/검증기를 사용하고 임의 코드 실행은 허용하지 않는다.
- 확정된 사건을 지역 뉴스·가격·NPC 대화로 표시. 각 단계에 timeout/fallback/diagnostics 제공.
- 검증: docs/18의 구매 및 TEST-E2E-001~006 시나리오. 플레이어가 방관해도 NPC 행동으로 사건이 종료되어야 한다.

**M5 게이트:** README의 첫 Vertical Slice 완주. 확정 경제 효과와 재시작 복구, 2클라이언트 일관성 포함.

### M6 — 개인 프롤로그와 공유 세계 진입 (TASK-012B)

선행: TASK-008/009/010/011/012A. 원본 TASK-012를 화면 구축과 이관으로 분리하여 복구 이전에 이관을 완성하는 순서 오류를 피한다.

- 개인 Region 생성, P0 입장/대화→P1 자원→P2 이동→P3 구조/전투→P4 보급→P5 공유 정착 흐름 구현.
- NOT_STARTED→IN_PROGRESS→READY_TO_TRANSFER→TRANSFERRING→COMPLETED를 영속 상태로 관리.
- transfer_id 및 보상 receipt로 캐릭터·NPC·아이템 이관을 멱등 처리. 단일 프로세스/동일 DB 범위만 지원.
- 이관 중 source/target 가시성과 쓰기 권한을 정하고, 장애 후 재조정까지 구현.
- 완료 후 재접속에서도 시작 버튼 숨김. NPC 사망·경로 실패·연결 해제에 대체 진행 경로 제공.
- 검증: TEST-PRO-001~006, 이관 재시도 100회 중복 0, 모든 단계 연결 해제/복구. 약 15분은 UX 가정이며 실제 플레이 결과로 조정.

### M7 — 부하·장애·운영 준비 (TASK-013~014)

**TASK-013: 계측·확장·장기 실행 — 선행 M5, 최종 인수는 M6 포함**

- 기존 단계의 Tick/큐/DB/복구 지표를 Prometheus/Grafana에 연결하고 trace로 명령 전체 경로 추적.
- 10명/NPC 1,000명 목표 환경, 하드웨어, seed, 행동 비율, release 빌드 조건 고정.
- Tick p95≤30ms/p99≤50ms, DB 결과 p95≤150ms, resync p95≤2초를 목표로 실측. 미달 시 병목과 조치 기록.
- 30분 부하→24시간 soak→필요 시 72시간으로 확대. fault matrix를 실제 프로세스/DB/연결에 적용.
- Active/Background 전환과 집계 규칙을 추가할 경우 자산 보존·시간 경과·활성 복귀를 별도 검증. 상세 NPC 부하 시험을 집계 모드로 대체하지 않는다.
- 5k/10k NPC, 100/500세션은 확장 실험으로 분리하고 MVP 완료 조건을 불필요하게 키우지 않는다.

**TASK-014: 배포·복구 훈련 — 선행 TASK-008/009/013, 대상 릴리스 M6 완료**

- 신규 환경용 Docker 이미지/Compose, TLS/WSS, secret 주입, 관리자 RBAC와 감사 로그 구성.
- health/readiness, drain, DB 장애·Worker 복구·이벤트 정체 runbook 작성.
- backup/PITR 복원 훈련, RPO/RTO 실측, 버전 변경·롤백 가능 범위 검증.
- 이미지 digest·DB migration·rules version·릴리스 체크리스트를 묶어 배포 후보 생성.
- 실제 서비스 공개/전환은 별도 배포 작업으로 수행한다. 계획 수립을 운영 전환 완료로 간주하지 않는다.

## 5. 착수 전에 해소할 설계 결정

아래는 계획상 제안이며 기존 OPEN 항목을 임의로 확정한 것이 아니다. 구현 담당 단계에서 ADR에 근거와 테스트를 기록한다.

| 결정 | 초기 제안 | 확정 시점 |
|---|---|---|
| 툴체인/라이브러리 버전 | Rust edition 2024, 호환 확인 후 버전 pin | TASK-001 |
| RNG·ID·해시 | 버전 고정 RNG, 외부 입력 기록, canonical 상태 | TASK-003 |
| protocol_version / schema_version | 통신 규약과 payload 스키마 의미 분리 | TASK-004 |
| 명령 상태 명칭 | 문서별 PENDING/PENDING_COMMIT 등 차이를 단일 상태 머신으로 정리 | TASK-004 |
| HP·사망 durability | 플레이어 사망/보상 durable, 일시 전투 상태 별도 | TASK-005 이전 |
| DB 버전·migration | 격리된 PostgreSQL 16+ 하나를 정확한 이미지 버전으로 고정 | TASK-007 |
| UUID·기존 데이터 | 새 월드·신규 계정 시작, 구 데이터 이식 후속 | TASK-002/007 |
| 인증·로그아웃 | 세션+1회 ticket, 로그아웃 후 무단 자동 이동 금지 | TASK-009 |
| 게임 시간 배율 | 10x 설정값으로 시작, 생산/소비 밸런스에서 검증 | TASK-010/011 |
| 프롤로그 transfer | 같은 DB의 멱등 이관 및 crash reconciliation | TASK-012B |
| 서버 사양·예산 | 실측 머신과 active 비율 명시 | TASK-013 이전 |
| snapshot/이벤트 보존·RPO/RTO | 복구 요구와 실제 저장량을 기준으로 결정 | TASK-014 이전 |

인증 방식·콘텐츠 UX처럼 제품 선택이 필요한 사항은 해당 단계에 구체 대안을 준비하고 사용자 결정을 받는다. 개발 환경·타입 분리·테스트 구성 등의 통상 구현 선택은 설계 기준 안에서 진행한다.

## 6. 작업 순서와 진행 관리

기본 순서: M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7.

TASK-010의 순수 AI 규칙은 M2 이후 개발 가능하지만 영속 기억·경제 통합은 M3 이후 완료한다. 계측·인증 검증·실패 테스트는 담당 기능과 함께 만들고 M7까지 미루지 않는다. 이 의존 관계는 동시 에이전트 실행 지시가 아니다.

각 티켓은 가능한 경우 계약/타입→정상 동작→실패·재시작→문서/증거 단위로 작게 나눈다. 작업 기록은 다음을 포함한다.

- TASK/REQ/TEST ID와 실제 변경 파일
- 상태: NOT STARTED / IN PROGRESS / BLOCKED / DONE
- 실행한 명령, 결과, 미실행 항목과 사유
- schema/rules/protocol 영향과 복구·롤백 제약
- 남은 작업 및 다음 착수 가능한 티켓

Rust 공통 검사 예정 명령:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

DB 통합·브라우저 E2E·부하 테스트는 별도 명령과 환경을 담당 티켓에서 추가한다. Golden hash는 실제 검증된 실행 후에만 기록하며 테스트 실패를 없애려고 자동 갱신하지 않는다. 달력 일정은 TASK-001~003 실측 후 산정한다.

## 7. 바로 다음 작업 묶음

1. TASK-000: SQL 추적 누락·ignore·문서 링크·신규 실행 경계 정리.
2. TASK-001: Rust 워크스페이스와 서버/Runner 기동, CI 기반.
3. TASK-002: 최소 ECS 월드와 StableId.
4. TASK-003: 10,000 Tick 재현성 게이트.

첫 작업 묶음의 데모는 GUI 없는 동일 시나리오 반복 실행과 동일 해시다. 이후 Command/이동/전투를 연결한다. 본 계획 작성 시 신규 엔진 구현·테스트·배포는 수행하지 않았다.

## 8. 근거 문서

- [README](../README.md): 첫 Vertical Slice, 품질 기준, 구현 불변식
- [요구사항](01-requirements.md), [아키텍처](02-hld-architecture.md)
- [저장·복구](09-persistence-recovery.md), [테스트·관측성](12-test-observability.md)
- [원본 TASK-001~014](13-implementation-backlog.md), [운영](14-operations-security.md)
- [ADR·OPEN 항목](15-adrs.md), [프롤로그](16-prologue-gameplay.md)
- [통합 시나리오](18-end-to-end-example.md)
