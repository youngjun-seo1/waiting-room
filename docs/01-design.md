# Waiting Room - 설계 문서

## 1. 개요

Cloudflare Waiting Room 스타일의 트래픽 관리 시스템.
티켓/상품 선착순 구매 시나리오에서 순간적인 트래픽 급증 시 FIFO 대기열로 사용자를 관리하고 오리진 서버를 보호하는 리버스 프록시.

### 요구사항

- FIFO 대기열 (선착순 입장, 대기 순번 표시, 예상 대기시간)
- 커스텀 대기 페이지 (진행률, 브랜딩)
- SSE 실시간 업데이트 + 자동 입장 리다이렉트
- 멀티 서버 수평 확장 (Redis 백엔드)
- Admin API (런타임 설정 변경)

### 기술 스택

- **언어**: Rust
- **웹 프레임워크**: axum 0.8
- **비동기 런타임**: tokio
- **상태 저장**: in-memory (개발) / Redis (프로덕션)
- **배포**: 단일 서버 또는 로드밸런서 뒤 멀티 서버

---

## 2. 아키텍처

### 단일 서버 (in-memory)

```
Client → [axum listener] → [Gate Middleware] → 입장 가능? → [Reverse Proxy] → Origin
                                    ↓ No
                            [Waiting Page + SSE]
                                    ↑
                          [Reaper] (세션 만료 → 다음 사용자 입장)
```

### 멀티 서버 (Redis)

```
              ┌─ WR Server 1 (8080) ─┐
Client → LB ──┤─ WR Server 2 (8081) ─┤── Redis ── Origin
              └─ WR Server N         ─┘
                                      ↕
                              Redis Pub/Sub (SSE 동기화)
```

---

## 3. 프로젝트 구조

```
waiting-room/
├── Cargo.toml
├── config.toml                # 기본 설정
├── src/
│   ├── main.rs                # 서버 부트스트랩, 백엔드 선택
│   ├── config.rs              # 설정 로딩 (TOML + 환경변수)
│   ├── state.rs               # Arc<AppState> 공유 상태
│   ├── backend.rs             # QueueBackend trait + MemoryBackend
│   ├── redis_backend.rs       # RedisBackend + Lua 스크립트
│   ├── pubsub.rs              # Redis Pub/Sub → 로컬 SSE 브릿지
│   ├── queue.rs               # FIFO 큐 자료구조 (WaitingQueue)
│   ├── session.rs             # HMAC-SHA256 서명 쿠키
│   ├── proxy.rs               # 리버스 프록시 (hyper 기반)
│   ├── middleware.rs           # Gate 미들웨어 (입장/대기 결정)
│   ├── admin.rs               # 관리 API
│   ├── waiting.rs             # 대기 페이지 핸들러 + SSE
│   ├── reaper.rs              # 세션 만료 + 자동 입장
│   ├── scheduler.rs           # 이벤트 스케줄러 (시간 기반 자동 제어)
│   └── templates/
│       └── waiting.html       # 대기 페이지 HTML
├── examples/
│   ├── origin.rs              # 테스트용 오리진 서버
│   ├── bench.rs               # 단일 서버 벤치마크
│   └── bench_multi.rs         # 멀티 서버 벤치마크 (모니터링 포함)
└── docs/
    ├── 01-design.md           # 설계 문서 (이 파일)
    ├── 02-implementation.md   # 구현 문서
    └── 03-testing.md          # 테스트 문서
```

---

## 4. 핵심 컴포넌트 설계

### 4.1 QueueBackend Trait

in-memory와 Redis 백엔드를 추상화하는 핵심 인터페이스.

```rust
#[async_trait]
pub trait QueueBackend: Send + Sync + 'static {
    async fn gate_check(&self, id: Option<SessionId>, new_id: SessionId,
                        max_active: u32, ttl_secs: u64) -> GateResult;
    async fn get_position(&self, id: &SessionId) -> Option<QueuePosition>;
    async fn is_active(&self, id: &SessionId) -> bool;
    async fn stats(&self) -> QueueStats;
    async fn flush(&self);
    async fn reaper_cycle(&self, ttl_secs: u64, max_active: u32) -> (usize, usize);
}
```

`gate_check()`이 미들웨어의 전체 판단 로직(is_active → touch, is_waiting → 위치, admit/enqueue)을 하나로 통합. Redis에서는 Lua 스크립트 1회 호출로 처리.

### 4.2 Gate Middleware Flow

```
1. 쿠키에서 세션 추출 (HMAC 검증)
2. gate_check() 호출 (1 round-trip)
   → Active: last_seen 갱신 → 프록시
   → Waiting: 대기 페이지 반환
   → Admitted: 새로 입장 → 프록시 + 쿠키 설정
   → Enqueued: 대기열 추가 → 대기 페이지 + 쿠키 설정
```

### 4.3 세션 관리

- HMAC-SHA256 서명 쿠키: `base64(session_id[16] | issued_at[8] | hmac[32])`
- JWT 대비 장점: 파싱 단순, 크기 작음, stateless (어느 서버에서든 검증 가능)

### 4.4 Redis 데이터 모델

| 용도 | Key | Type | 설명 |
|------|-----|------|------|
| Active 세션 | `wr:active` | Hash | field=session_uuid, value=admitted_at_ms |
| Active TTL | `wr:active:{id}:ls` | String+TTL | `touch()`마다 `EXPIRE` 리셋 |
| 대기열 (FIFO) | `wr:waiting` | Sorted Set | score=timestamp_ms, member=uuid |
| ETA 통계 | `wr:stats` | Hash | `total_active_duration_ms`, `completed_sessions` |
| Reaper 리더 락 | `wr:reaper:lock` | String+NX+EX | 단일 리더 보장 |
| SSE 알림 | `wr:notify` | Pub/Sub | 큐 변경 시 전 서버 알림 |

### 4.5 Lua 스크립트 (Redis 원자성 보장)

1. **gate_check.lua**: 미들웨어 핫패스 전체를 1 round-trip으로 처리
2. **reaper.lua**: 만료 세션 정리 + 대기열에서 입장 (atomic)
3. **flush.lua**: 전체 초기화

### 4.6 Reaper (세션 만료)

- `tokio::time::interval`로 주기적 실행 (기본 1초)
- in-memory: 직접 실행
- Redis: `SET wr:reaper:lock NX EX 10`으로 리더 선출, 1대만 실행
- 만료된 active 세션 정리 → 대기열에서 다음 사용자 입장 → SSE 알림

### 4.7 SSE 크로스서버 동기화

```
Reaper/Admin → PUBLISH wr:notify → Redis Pub/Sub
                                     ↓
각 서버의 pubsub listener → 로컬 sse_tx.send(()) → BroadcastStream → SSE 클라이언트
```

기존 SSE 핸들러는 로컬 `BroadcastStream`만 사용하므로 변경 없음.

### 4.8 스케줄러 (이벤트 시간 기반 제어)

특정 시간에 대기실을 자동으로 제어하는 기능. 쿠폰 선착순 등 이벤트 운영에 사용.

**3단계 라이프사이클:**

```
[pending] ──enable_at──→ [queuing] ──start_at──→ [active] ──disable_at──→ [ended]
                          대기열 수집만            순차 입장 시작           대기실 종료
                          (입장 차단)             (reaper 동작)           (트래픽 직통)
```

**구현 방식:**
- `scheduler.rs`: 1초마다 스케줄 목록을 확인, phase 전환 시 `config.enabled`와 `config.schedule_started`를 자동 변경
- Queuing phase: `max_active`를 0으로 설정 → 모든 요청이 대기열로
- Active phase: `max_active`를 스케줄에 설정된 값으로 복원 → 대기열에서 순차 입장
- Admin API로 스케줄 CRUD 가능

---

## 5. 설정

### config.toml

```toml
listen_addr = "0.0.0.0:8080"
origin_url = "http://127.0.0.1:3000"
max_active_users = 1000
session_ttl_secs = 300
queue_cookie_name = "__wr_token"
admin_api_key = "change-me-in-production"
enabled = true
redis_url = ""  # 비어있으면 in-memory, "redis://..." 이면 Redis 모드
```

### 환경변수 오버라이드

| 환경변수 | 설명 |
|---------|------|
| `WR_LISTEN_ADDR` | 서버 주소 |
| `WR_ORIGIN_URL` | 오리진 서버 URL |
| `WR_MAX_ACTIVE_USERS` | 최대 동시 입장 수 |
| `WR_SESSION_TTL_SECS` | 세션 TTL |
| `WR_ADMIN_API_KEY` | 관리 API 키 |
| `WR_REDIS_URL` | Redis 연결 URL |
| `WR_ENABLED` | 대기실 활성화 여부 |

---

## 6. API 엔드포인트

### Public

| Method | Path | 설명 |
|--------|------|------|
| `*` | `/*` | Gate 체크 후 프록시 또는 대기 페이지 |
| `GET` | `/__wr/events` | SSE 스트림 (순번 실시간 업데이트) |
| `GET` | `/__wr/status` | 큐 상태 JSON |

### Admin (X-Api-Key 헤더 인증)

| Method | Path | 설명 |
|--------|------|------|
| `GET` | `/__wr/admin/config` | 현재 설정 조회 |
| `PUT` | `/__wr/admin/config` | 런타임 설정 변경 |
| `POST` | `/__wr/admin/enable` | 대기실 활성화 |
| `POST` | `/__wr/admin/disable` | 대기실 비활성화 |
| `GET` | `/__wr/admin/stats` | 상세 통계 |
| `POST` | `/__wr/admin/flush` | 큐 초기화 |
| `GET` | `/__wr/admin/schedules` | 스케줄 목록 조회 |
| `POST` | `/__wr/admin/schedules` | 스케줄 등록 |
| `DELETE` | `/__wr/admin/schedules/{id}` | 스케줄 삭제 |
