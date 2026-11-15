# 15. 아키텍처 결정 기록(ADR) 및 미결 사항

**한국어** | [English](en/15-adrs.md)

## 1. 결정 기준선

| ADR | 상태 | 결정 | 근거/대안 |
|---|---|---|---|
| ADR-001 | 채택 | Rust 엔진/서버, Bevy ECS, Tokio, Axum | ECS/메모리 관리·I/O 분리. 대안 Go 단일 서버 |
| ADR-002 | 채택 | Region별 ECS World + single writer | 상태 충돌 억제. 대안 global ECS lock |
| ADR-003 | 채택 | 10Hz fixed Tick + game clock 분리 | 검증 용이. Tick rate는 측정에 따라 조정 |
| ADR-004 | 채택 | 초기 SingleThreaded ECS schedule | 결정성 재현 우선. 병렬 ECS는 후속 |
| ADR-005 | 채택 | Command/Event/StateDelta 분리 | 의도와 사실, UI 표현을 혼동하지 않음 |
| ADR-006 | 채택 | Durable DB Commit ACK 전 최종 성공 금지 | 중복 지급/phantom asset 방지 |
| ADR-007 | 채택 | PostgreSQL receipt + event + outbox 단일 TX | 원자성/관찰 가능성 |
| ADR-008 | 채택 | Snapshot + durable event/reconciliation | 변경 종류별 복구 보장 구분 |
| ADR-009 | 채택 | Epoch 기반 DB fencing | stale Region Worker 쓰기 차단 |
| ADR-010 | 채택 | Utility AI + GOAP, LLM 보조 | Tick 예측가능성과 자연어 유연성 |
| ADR-011 | 채택 | MVP JSON WebSocket → 필요시 Protobuf | 초기 개발 속도 |
| ADR-012 | 채택 | 단일 프로세스 먼저, Region Worker Pool 후 확장 | 분산 복잡도 최소화 |
| ADR-013 | 채택 | Stable entity ID는 UUID | ECS 런타임 핸들과 분리 |
| ADR-014 | 채택 | 액션·돈·좌표 계산은 정수/fixed-point | 재현/정합성 향상 |
| ADR-015 | 채택 | 지역 간 즉시 거래 MVP 제외 | 다중 Region 원자적 상태 관리 연기 |

## 2. 구현 중 반드시 결정할 항목

| ID | 항목 | 현재 가정 | 결정 담당/기한(프로젝트에서 지정) |
|---|---|---|---|
| OPEN-001 | 엔진 성능 목표/하드웨어 | p95 30ms/1k NPC | Tech Lead / 벤치마크 전 |
| OPEN-002 | 실시간 게임 시간 배율 | 10x | Game Designer / 콘텐츠 작성 전 |
| OPEN-003 | HP/사망 상태 durability | 플레이어 사망 durable | Game/Server / 전투 구현 전 |
| OPEN-004 | 플레이어 로그아웃 시 행동 | 접속 해제 후 NPC처럼 자동 이동 금지 | Game Designer / Multiplayer 구현 전 |
| OPEN-005 | Realm/World 소유와 멀티 지역 경계 | 단일 World, 1 Region | Architect / Region 이관 구현 전 |
| OPEN-006 | RNG 구현체 및 버전 | 고정 알고리즘 + 안정 해시 | Core Engineer / Golden Test 전 |
| OPEN-007 | 이벤트·스냅샷 보존기간 | 영속 이벤트 장기 보존 | Operations / 운영 전 |
| OPEN-008 | 계정/인증 방식 | HTTPS ticket + session | Security/Backend / WS 공개 전 |
| OPEN-009 | Region 크기/AOI 비용 | Region 64×64, AOI cell 16×16 | Game/Server / 부하 테스트 |
| OPEN-010 | SQL 마이그레이션 전략/DB 버전 | PostgreSQL 16+ | DBA/Backend / DB 최초 적용 전 |
| OPEN-011 | 저속 클라이언트 disconnect 한계 | 2초 backlog 가정 | Backend / WS 부하 테스트 |
| OPEN-012 | 개인 프롤로그 공유세계 이주 | idempotent transfer | Game Designer / 프롤로그 MVP 전 |

## 3. 변경 관리 절차

1. 변경 제안은 ADR 번호를 새로 발급하고 *문제/선택지/결정/영향/되돌리기 방법* 기록.
2. 변경이 `Tick`, `이벤트 스키마`, `영속성`, `Region ownership`에 영향을 주면 deterministic/recovery tests 갱신.
3. 주요 데이터 경계 변경에는 데이터 마이그레이션과 구버전 클라이언트 호환 정책을 반드시 포함.
4. 한 번 결정된 값이라도 실제 벤치마크/장애 테스트 결과가 다르면 변경 가능하나 이유와 영향 문서를 남긴다.

## 4. ADR 템플릿

```markdown
## ADR-XXX: 제목
- Status: proposed / accepted / deprecated
- Context: 해결할 문제와 범위
- Decision: 정확한 선택과 적용 시점
- Alternatives: 대안과 배제 이유
- Consequences: 성능/복잡도/운영 영향
- Verification: 통과해야 하는 TEST ID
- Migration/Rollback: 기존 시스템 전환 절차
```
