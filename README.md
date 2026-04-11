# Waiting Room

Cloudflare Waiting Room 스타일의 트래픽 관리 리버스 프록시. 선착순 이벤트 시 FIFO 대기열로 사용자를 관리하고 오리진 서버를 보호합니다.

## 주요 기능

- FIFO 대기열 (선착순 입장, 대기 순번, 예상 대기시간)
- SSE 실시간 업데이트 + 자동 입장 리다이렉트
- 스케줄 기반 자동 제어 (start_at → end_at, 이벤트별 origin_url 지정)
- 스케줄 종료 시 대기열 자동 flush + 종료 안내 SSE 이벤트
- 대기실 비활성화 시 "이벤트 참여 시간이 아닙니다" 안내 페이지
- In-memory / Redis 백엔드 (멀티 서버 수평 확장)
- Redis 모드 시 스케줄 영속화 (멀티 서버 간 스케줄 공유)
- Admin SPA (React) + Admin API

## 빠른 시작

```bash
# 전체 서버 한 번에 시작
./start.sh

# 또는 개별 실행
cargo run --example origin          # 오리진 서버 (테스트용)
cargo run                            # Waiting Room 서버
cd admin && npm install && npm run dev  # Admin SPA
```

- Waiting Room: http://localhost:8080
- Admin SPA: http://localhost:5173

```bash
./stop.sh      # 전체 서버 종료
./status.sh    # 서버 실행 상태 확인
```

## 설정

`config.toml` 또는 환경변수로 설정:

| 설정 | 환경변수 | 기본값 | 설명 |
|------|---------|--------|------|
| `listen_addr` | `WR_LISTEN_ADDR` | `0.0.0.0:8080` | 서버 주소 |
| `origin_url` | `WR_ORIGIN_URL` | `http://127.0.0.1:3000` | 오리진 서버 URL |
| `max_active_users` | `WR_MAX_ACTIVE_USERS` | `100` | 최대 동시 입장 수 |
| `session_ttl_secs` | `WR_SESSION_TTL_SECS` | `300` | 세션 TTL (초) |
| `admin_api_key` | `WR_ADMIN_API_KEY` | - | Admin API 인증 키 |
| `redis_url` | `WR_REDIS_URL` | `""` (in-memory) | Redis URL (멀티 서버) |
| `enabled` | `WR_ENABLED` | `false` | 대기실 활성화 (스케줄/API로 제어) |

## 멀티 서버 (Redis)

```bash
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8080" cargo run
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8081" cargo run
```

Redis 모드에서는 대기열과 스케줄이 모두 Redis에 저장되어 멀티 서버 간 공유됩니다.

## 스케줄

Admin SPA 또는 API로 시간 기반 자동 제어:

```bash
curl -X POST -H "X-Api-Key: ..." -H "Content-Type: application/json" \
  -d '{
    "name": "쿠폰 이벤트",
    "start_at": "2026-04-15T10:00:00Z",
    "end_at": "2026-04-15T11:00:00Z",
    "max_active_users": 100,
    "origin_url": "http://event-server:3000"
  }' http://localhost:8080/__wr/admin/schedules
```

- `start_at`에 대기실 자동 ON + origin_url 전환
- `end_at`에 자동 OFF + 대기열 flush + 종료 안내

## 기술 스택

- Rust + axum + tokio
- Redis (선택, 멀티 서버 시)
- React + TypeScript + Vite (Admin SPA)

## 문서

- [설계](docs/01-design.md)
- [구현](docs/02-implementation.md)
- [테스트](docs/03-testing.md)
- [Q&A](docs/04-qna.md)
- [Cloudflare 비교 분석](docs/05-comparison.md)
