# M0 개발 환경 결정

**한국어** | [English](en/21-m0-bootstrap-decisions.md)

상태: M0 구현 결정. 기존 ADR-001~015를 변경하지 않는다. 아래 실행 상태는 M0 당시 기록이며 현재 동작은 루트 README를 참고한다.

## ADR-016: 신규 실행 경계와 도구 고정

- Rust 1.90.0, edition 2024, Cargo resolver 3을 고정한다.
- Axum 0.8.6 / Tokio 1.48.0 / Bevy ECS 0.17.2 / SQLx 0.8.6과 직접 의존성을 정확한 버전으로 고정하고 Cargo.lock으로 전이 의존성도 재현한다.
- 최신 버전 추종보다 이 조합의 컴파일·테스트 증거를 우선한다. 업데이트는 별도 변경으로 검증한다.
- game-domain과 simulation 계열은 네트워크/SQLx 의존성이 없다. persistence/networking은 별도 crate다.
- 서버는 live=200, ready=503으로 시작한다. Region·DB가 없는 M0를 운영 준비 완료로 표시하지 않는다.
- Runner는 --check만 성공하며 executed_ticks=0이다. 실제 실행 요청은 미구현 오류로 끝낸다.
- Windows 도구 설치 대신 대상 Linux 서버의 격리된 Rust Docker 이미지로 검증한다.
- 신규 개발 DB는 별도 Compose project/DB/volume을 사용하고 127.0.0.1:55432에만 바인딩한다.
- 구 게임의 루트 Compose는 참고 자료로 보존한다. 구 서비스 재시작·데이터 이식은 수행하지 않는다.

## 설정과 비밀 경계

- 명시한 TOML 파일을 읽은 뒤 WSE_* 환경 변수로 덮어쓰고 검증한다.
- 파일 누락/알 수 없는 키/잘못된 수치/지원하지 않는 프로토콜은 시작 오류다.
- WSE_HTTP_LISTEN, WSE_WS_AUTH_TIMEOUT_SEC, WSE_TICK_RATE_HZ, WSE_GAME_TIME_SCALE,
  WSE_REGION_CMD_CAP, WSE_DURABLE_CAP, WSE_SNAPSHOT_INTERVAL_SEC,
  WSE_REGION_LEASE_SEC, WSE_LEASE_RENEW_SEC를 지원한다.
- 기타 설정은 TOML로 관리한다. DATABASE_URL은 TASK-007부터 사용하며 M0에서 로깅하거나 접속하지 않는다.
- 기본 서버 listen은 개발 안전성을 위해 127.0.0.1:18080으로 설정한다.
- health endpoint는 프로세스 상태만 알리며 게임 데이터·비밀값은 반환하지 않는다.

## 문서 편집과 검증

README.md, README.en.md, docs/NN-*.md, docs/en/NN-*.md, schemas가 편집 원본이다. [문서 관리 안내](README.md)의 명령으로 통합본과 manifest를 생성하며, 생성물을 직접 편집하지 않는다.

과거 M0 명령은 `python3 scripts/sync-design-docs.py`와 `--check` 옵션이었다. 이 Python 스크립트는 현재 체크아웃에 없으므로 관리 안내의 Node.js 명령을 사용한다.

manifest는 생성 시각 대신 원본 byte/hash를 기록하여 동일 원본에서 동일 결과를 만든다. SQL은 새 DB에서 검증하기 전까지 설계 초안이며, Git 포함과 실행 검증은 구분한다.

과거 CI 정책은 nmain 대상 PR 및 nmain/codex 브랜치 push였다. 워크플로 정의와 원격 실행 PASS는 별도 상태다.
