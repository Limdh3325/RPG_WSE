# 한국어 문서

**한국어** | [English](en/README.md)

설치와 현재 구현 제한은 [프로젝트 README](../README.md)를 먼저 읽어 주세요. 00–21 문서는 설계 계약과 과거 단계별 기록을 포함하며, 계획된 기능이 구현 완료를 뜻하지는 않습니다.

| 문서 | 주제 |
|---|---|
| [00-vision-scope.md](00-vision-scope.md) | 00. 비전 · 게임 범위 · 용어 |
| [01-requirements.md](01-requirements.md) | 01. 요구사항 명세 (SRS) |
| [02-hld-architecture.md](02-hld-architecture.md) | 02. 전체 아키텍처 (HLD) |
| [03-core-tick.md](03-core-tick.md) | 03. 시뮬레이션 코어와 Fixed Tick (LLD) |
| [04-region-ecs.md](04-region-ecs.md) | 04. Region Worker · ECS · Ownership (LLD) |
| [05-command-event.md](05-command-event.md) | 05. Command / Event Pipeline · 실행 규칙 (LLD) |
| [06-npc-ai.md](06-npc-ai.md) | 06. NPC Decision Engine — Utility AI + GOAP (LLD) |
| [07-world-systems.md](07-world-systems.md) | 07. World Systems — 이동·전투·경제·세력·환경·사건 (LLD) |
| [08-data-erd.md](08-data-erd.md) | 08. 영속 데이터 모델 · ERD (LLD) |
| [09-persistence-recovery.md](09-persistence-recovery.md) | 09. Persistence · Transaction · Snapshot · Recovery (LLD) |
| [10-network-replication.md](10-network-replication.md) | 10. 멀티플레이어 동기화 · Network/AOI (LLD) |
| [11-protocols.md](11-protocols.md) | 11. 외부 API와 프로토콜 계약 (LLD) |
| [12-test-observability.md](12-test-observability.md) | 12. 테스트 전략 · 벤치마크 · 관측성 |
| [13-implementation-backlog.md](13-implementation-backlog.md) | 13. 구현 백로그 · 작업 지시서 (실행 계획) |
| [14-operations-security.md](14-operations-security.md) | 14. 운영 · 보안 · 배포 · 장애 대응 |
| [15-adrs.md](15-adrs.md) | 15. 아키텍처 결정 기록(ADR) 및 미결 사항 |
| [16-prologue-gameplay.md](16-prologue-gameplay.md) | 16. 플레이어 경험 · 프롤로그 · 공유 세계 진입 설계 |
| [17-developer-onboarding.md](17-developer-onboarding.md) | 17. 신규 개발자 온보딩 · 작업 시작 가이드 |
| [18-end-to-end-example.md](18-end-to-end-example.md) | 18. 끝까지 따라가는 구현 예제 — 구매와 식량 부족 |
| [19-nmain-development-plan.md](19-nmain-development-plan.md) | nmain 신규 개발 상세 실행 계획 |
| [20-development-status.md](20-development-status.md) | 개발 상태와 요구사항 추적 |
| [21-m0-bootstrap-decisions.md](21-m0-bootstrap-decisions.md) | M0 개발 환경 결정 |

## 두 언어 문서 관리

- 한국어: 루트 README.md와 docs/NN-*.md. 영어: 루트 README.en.md와 docs/en/NN-*.md.
- 파일명, REQ/ADR/TASK/TEST ID, 수치 제한, 명령, 프로토콜 필드, 인수 기준을 유지합니다. 설명과 주석을 번역하고 실행 식별자는 바꾸지 않습니다.
- 변경 시 두 언어를 함께 갱신하고 언어 전환 링크를 유지합니다. 과거 단계의 기록을 현재 완료 상태로 바꾸지 않습니다.
- 코드·스키마는 공통 파일을 사용합니다. 통합 문서는 원본 SQL/TOML과 원래 주석을 그대로 포함합니다.
- 저장소 루트에서 Node.js로 실행합니다. 별도 패키지 설치는 필요 없습니다.

```sh
node scripts/sync-design-docs.mjs
node scripts/sync-design-docs.mjs --check
```

RPG_WSE_Complete_Design.md, RPG_WSE_Complete_Design.en.md, manifest.json을 재생성하고 언어별 파일 대응·로컬 링크·코드 블록·요구사항 ID·생성물 최신 여부를 검사합니다. 예제 실행이나 번역 의미의 완전한 일치를 자동 보증하지는 않습니다. 생성물은 직접 편집하지 마세요.
