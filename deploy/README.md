# WSE 신규 개발 환경

기존 루트 docker-compose.yml은 이전 Go 게임 참조이며 WSE 실행에 사용하지 않는다.
WSE Compose는 프로젝트명 wse-m0-dev, DB wse_dev, 전용 볼륨, loopback 55432를 사용한다.

```sh
cp deploy/.env.example deploy/.env
# deploy/.env의 개발 비밀번호를 변경한다.
docker compose --env-file deploy/.env -f deploy/compose.dev.yml up -d --wait
docker compose --env-file deploy/.env -f deploy/compose.dev.yml ps
docker compose --env-file deploy/.env -f deploy/compose.dev.yml down
```

down은 데이터 볼륨을 삭제하지 않는다. 운영 서비스의 DB/볼륨은 사용하지 않는다.
M0에서 DB 기동만 확인하며 초기 SQL은 자동 마운트하지 않는다.
초기 SQL의 실제 빈 DB 검증과 migration 연결은 TASK-007에서 수행한다.

서버는 아직 DB를 사용하지 않으며 /health/ready는 항상 503이다.
M0에서는 게임서버를 상시 운영 서비스로 등록하지 않는다.

## Docker로 Rust 검증

호스트에 Rust를 설치하지 않고 다음 명령으로 동일 버전을 사용할 수 있다.
이 이미지는 Python 3도 제공한다. .local-tools 같은 Windows 도구는 필요하지 않다.

```sh
docker run --rm \
  -v "$PWD:/workspace" \
  -v wse-m0-cargo-cache:/usr/local/cargo/registry \
  -w /workspace rust:1.90.0-bookworm \
  sh -c 'rustup component add rustfmt clippy && sh scripts/verify-m0.sh'
```

Cargo.lock은 저장소에 포함한다. 최초 의존성 변경 때만 lock을 갱신한다.
정확한 이미지 digest는 M0 완료 보고서에 기록한다.

## Nginx 개발 프록시

`nginx.conf`의 PID와 정적 파일 경로는 Nginx prefix 기준 상대 경로입니다. 먼저 `apps/web-client`에서 `npm ci`와 `npm run build`를 실행하고, 저장소 루트에서 다음 명령을 실행하세요(Linux/macOS 셸 기준).

```sh
nginx -t -p "$PWD/" -c deploy/nginx.conf
nginx -p "$PWD/" -c deploy/nginx.conf
# 종료
nginx -p "$PWD/" -c deploy/nginx.conf -s quit
```

서버는 `127.0.0.1:18080`, Nginx는 포트 8000을 사용합니다. `/etc/nginx/mime.types`는 배포판 기본 경로이므로 설치 환경에 따라 조정하세요. `deploy/nginx.pid`는 실행 중 생성되는 로컬 파일이며 Git에서 제외합니다.
