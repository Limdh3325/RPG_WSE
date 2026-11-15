# 01. 요구사항 명세 (SRS)

## 1. 기능 요구사항

| ID | 등급 | 요구사항 | 인수 기준 |
|---|---|---|---|
| REQ-F-001 | P0 | 클라이언트 없는 Headless 실행 | `run --seed X --ticks 10000` 가능 |
| REQ-F-002 | P0 | Region ECS/안정 ID | 재시작 이후 동일 Game Entity UUID 복원 |
| REQ-F-003 | P0 | 고정 10Hz Tick 및 시간 배율 분리 | Tick/WorldTime 상호 독립 제어 |
| REQ-F-004 | P0 | Command 검증·멱등성 | 동일 구매 Command 재전송 시 1회 확정 |
| REQ-F-005 | P0 | Region Single Writer | 동일 Epoch에 2 Worker 동시 Commit 불가 |
| REQ-F-006 | P0 | 게임 시간 동안 NPC 자율행동 | 입력 없어도 NPC 상태·작업이 변화 |
| REQ-F-007 | P0 | 이동/충돌/경로탐색 | 도달 불가 시 실패·재경로 사유 기록 |
| REQ-F-008 | P0 | 기본 전투/피해/사망 | HP·충돌·범위·쿨다운 검증 |
| REQ-F-009 | P0 | 이벤트 규칙·후속 발생 | 사건 ID로 인과관계 추적 가능 |
| REQ-F-010 | P0 | 영속 상태/재시작 복구 | Commit한 자산이 재시작 후 유지 |
| REQ-F-011 | P0 | WS 클라이언트 상태 동기화 | 2개 클라이언트 최신 Snapshot 일치 |
| REQ-F-012 | P1 | Region Active/Background | 유휴 영역의 NPC 집계 시뮬레이션 |
| REQ-F-013 | P1 | 식량 생산·소비와 시장 가격 | 재고 부족에 따른 가격/이벤트 발생 |
| REQ-F-014 | P1 | NPC 관계·기억·목표 | 구조화된 기억이 의사결정에 반영 |
| REQ-F-015 | P1 | 게임 세계 사망·경제 효과 | 월드 이력/평판/재고 일관 반영 |
| REQ-F-016 | P1 | 개인 프롤로그 → 공유 월드 | 중복 캐릭터/아이템 없이 1회 이전 |
| REQ-F-017 | P1 | 운영 도구 | Region pause/resume, health, diagnostics |
| REQ-F-018 | P2 | LLM 대화/행동 제안 | 규칙 검증 전에는 세계 상태 변경 불가 |
| REQ-F-019 | P2 | 다중 서버 Region 이관 | Epoch 기반 안전한 권한 양도 |

## 2. 비기능 요구사항

| ID | 요구사항 | 초기 목표 / 검증 방식 |
|---|---|---|
| REQ-N-001 | 재현 가능성 | 엔진/규칙 버전·Seed·입력 로그 동일 시 이벤트/상태 해시 동일 |
| REQ-N-002 | Tick 성능 | 10Hz, Active Region p95 ≤30ms, p99 ≤50ms (**목표치**) |
| REQ-N-003 | 부하 | 1,000 NPC/Region 기준 측정, 10,000 NPC는 분산/집계 실험 |
| REQ-N-004 | 내결함성 | Commit 후 ACK 손실 재처리에도 중복 거래 없음 |
| REQ-N-005 | 메모리 상한 | 채널 bounded, 적체 시 응답·폐기 정책 명시 |
| REQ-N-006 | 보안 | 서버 권한, Actor 소유권, WSS/TLS, 운영 API RBAC |
| REQ-N-007 | 관측성 | Tick, 큐, 오류, DB, 복구, 이벤트, Region 상태 지표 |
| REQ-N-008 | 진화 가능성 | JSON/Protobuf/DB schema_version, 마이그레이션 정책 |
| REQ-N-009 | 데이터 보존 | 영속 이벤트는 유실 없이, 위치는 마지막 성공 checkpoint까지 |

## 3. 전제 및 제약

- 단일 Region+단일 서버에서 시작. Phase 1에 분산 합의/Kafka 불필요.
- 게임 상태 변경은 **모두 서버**에서 검증; 클라이언트는 렌더링 주체.
- 서버는 Rust; UI는 Vue3 + Phaser; LLM은 비동기 별도 어댑터.
- **PostgreSQL 16+ 가정**, 3차원/높낮이·연속 물리는 MVP 범위 밖.
- Region 단위 컴포넌트 상태는 메모리 ECS가 소유; 재산 DB가 최종 확정 근거.

## 4. 개발 완료 정의 (공통 DoD)

1. RFC/ADR 변경이 문서에 반영됨.
2. 코드·마이그레이션·메시지 스키마에 버전 관리 적용.
3. 정상/실패/중복/재시작 경로의 자동화 테스트 존재.
4. `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` 통과.
5. Grafana/Prometheus에서 주요 지표 확인 가능.
6. 담당 기능의 로그에 `world_id`, `region_id`, `tick`, `command_id`, `trace_id` 중 해당 ID가 남음.
7. 통과하지 못한 성능 목표는 실제 측정값과 후속 작업으로 명시(허위 완료 처리 금지).
