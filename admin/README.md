# Waiting Room Admin SPA

React + TypeScript + Vite 기반 관리 대시보드.

## 실행

```bash
npm install
npm run dev       # 개발 서버 (http://localhost:5173)
npm run build     # 프로덕션 빌드
```

## 페이지

| 페이지 | 기능 |
|--------|------|
| **Login** | Admin API Key 입력 (localStorage 저장) |
| **Dashboard** | 실시간 큐 상태 모니터링, 설정 변경, Enable/Disable 토글 |
| **Schedules** | 스케줄 등록/삭제, phase 실시간 표시 |

## 스케줄 등록

- **Name**: 스케줄 이름 (예: 쿠폰 이벤트)
- **Start At**: 대기실 활성화 시점 (기본값: 현재 시간)
- **End At**: 대기실 종료 시점 (기본값: 현재 시간, 반드시 Start At보다 미래)
- **Max Active Users**: 최대 동시 입장 수 (기본값: 100)

스케줄이 `start_at`에 도달하면 자동으로 대기실 ON, `end_at`에 도달하면 자동으로 대기실 OFF.

## API 연결

Vite dev server가 `/__wr/*` 요청을 Waiting Room 서버(8080)로 프록시합니다.
`vite.config.ts`에서 proxy 설정 확인.

## 기술 스택

- React 19 + TypeScript
- Vite (빌드/개발 서버)
- Tailwind CSS (스타일링)
