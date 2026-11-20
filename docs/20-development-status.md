# 개발 상태와 요구사항 추적

**한국어** | [English](en/20-development-status.md)

기준: nmain 신규 엔진. 이 표는 원문 작성 시점의 기록이며 이후 변경은 루트 README와 실제 구현을 확인한다. 기존 게임의 테스트 결과를 신규 엔진 완료 증거로 사용하지 않는다.

| TASK | REQ | 검증 | 상태 |
|---|---|---|---|
| 000 저장소 | N-008 | SQL/설정 Git 포함, 문서 링크·manifest check | DONE (M0 로컬 게이트) |
| 001 환경 | F-001 기반, N-007 기반 | fmt/clippy/test/build, 실제 HTTP/CLI smoke, CI 정의 | DONE (M0 로컬 게이트) |
| 002 ECS | F-002 | Stable ID 생성/삭제/복원, 10,000 Entity 수명주기, 중복 거부 | DONE (로컬 게이트 통과) |
| 003 Tick | F-001/003, N-001 | 10,000 Tick 상태·이벤트 hash | DONE (로컬 게이트 통과) |
| 004 계약 | F-004/009, N-008 | 중복/순서/버전/규칙 순환 | DONE (로컬 게이트 통과) |
| 005 이동/전투 | F-007/008/015 | 충돌/경로/사거리/사망 | DONE (로컬 게이트 통과) |
| 006 Worker | F-005/017, N-005 | single writer, bounded queue, 종료 | NOT STARTED |
| 007 영속 거래 | F-004/005/010, N-004/009 | 실제 PG, Commit/ACK failpoint | NOT STARTED |
| 008 복구 | F-010, N-009 | checksum/재시작/receipt/RNG | NOT STARTED |
| 009 통신 | F-011, N-006/008 | 2 Client/위조/gap/reconnect | NOT STARTED |
| 010 NPC | F-006/014 | 무입력 생활/계획 실패 복구 | NOT STARTED |
| 011 경제 | F-009/013/015 | 부족→수송대→습격→경제 결과 | NOT STARTED |
| 012A 화면 | F-007/008/011 | 실제 브라우저 E2E | NOT STARTED |
| 012B 프롤로그 | F-016 | TEST-PRO-001~006 | NOT STARTED |
| 013 계측/부하 | F-012, N-002/003/005/007 | 지표/부하/soak/Background 정합성 | NOT STARTED |
| 014 운영 | F-017, N-006/009 | 백업 복원/배포/runbook | NOT STARTED |
| 후속 LLM | F-018 | 비권한 제안, timeout/fallback | DEFERRED |
| 후속 분산 | F-019 | 다중 서버 Region 이관 | DEFERRED |

REQ 표의 F/N 접두사는 docs/01의 REQ-F/REQ-N을 뜻한다. 테스트 구체 ID는 담당 TASK에서 연결한다.
M0의 Runner --check는 CLI 검증이며 REQ-F-001의 헤드리스 시뮬레이션 완료가 아니다.
M0에서는 Tick 지표나 저장 지표를 거짓 값으로 노출하지 않는다.

과거 완료 근거: `outbox/2026-11-09-m0-completion.md` (현재 체크아웃에 포함되지 않음). GitHub 호스팅 CI는 커밋/push 전이므로 NOT RUN.
