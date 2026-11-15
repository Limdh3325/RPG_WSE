# RPG WSE Web Client (Vue 3 + TypeScript + Phaser)

RPG World Simulation Engine의 M4/M5 클라이언트 구현입니다.
Axum WebSocket 게이트웨이(`TASK-009`) 및 `shared-protocol` v1 규격과 연동됩니다.

## 주요 기능
- **Wire Protocol v1 연동**: Ticket Auth, WorldSnapshot, StateDelta (sequence gap 감지 및 ResyncRequest 복구), Client Commands (Move, Attack, etc.)
- **Phaser 3 월드 뷰**: 타일 그리드 렌더러, 카메라 팔로우, 엔티티 스프라이트 및 동적 HP 바/이름표
- **Top HUD**: 실시간 연결 상태, Ping RTT 지연, Region ID, Tick 표시
- **인터랙션**: 마우스 타일 클릭 이동, WASD/방향키 카메라/이동 조작, 퀵 액션 바
- **실시간 피드**: AOI 시야 내 엔티티 실시간 감시 목록 및 네트워크/시스템 이벤트 로그

## 실행 및 빌드
```bash
# 의존성 설치
npm install

# 로컬 개발 서버 (포트 3000)
npm run dev

# 단위 테스트 (Vitest)
npm test

# 프로덕션 빌드 & 타입 체크
npm run build
```

## 그래픽 에셋 출처 및 사용 조건 (Asset Provenance & License)
- **월드 타일 및 그리드**: Phaser 3 내장 벡터 그래픽스 파이프라인(`Phaser.GameObjects.Graphics`)에 의해 런타임 프로시저럴 생성 (32x32 그리드, 축 정렬 가이드). 라이선스: MIT.
- **캐릭터 및 몬스터 엔티티**: 런타임 벡터 지오메트리 렌더링(Green: 플레이어, Blue: 우호 NPC, Red: 적 몬스터, Purple: 타 플레이어) 및 네이티브 텍스트 렌더러. 라이선스: MIT.
- **UI 이모지 아이콘(🌿, 🪵, 🧪, 🗡️, 🪙, 📦 등)**: Unicode 표준 시스템 기본 이모지 폰트 세트 (Noto Color Emoji / Apple Color Emoji / Segoe UI Emoji). 라이선스: OS 기본 내장/SIL Open Font License.
- **상용 및 배포 조건**: 현재 프로시저럴 플레이스홀더 단계에서는 외부 유료/저작권 제한 에셋이 없으며, 추후 정식 2D/3D 스프라이트 시트 도입 시 에셋 스토어 라이선스 및 사용 조건을 본 문서에 갱신 기록합니다.
