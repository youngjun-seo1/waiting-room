# Waiting Room - 구현 문서

## 1. 구현 단계

### Phase 1: 프로젝트 초기화 + 기본 구조

- `cargo init`으로 Rust 프로젝트 생성
- 의존성 설정: axum, hyper, tokio, parking_lot, hmac, sha2, uuid, minijinja, serde, tracing
- `config.rs`: TOML 파일 + 환경변수 오버라이드로 설정 로딩
- `config.toml`: 기본 설정 파일

### Phase 2: 핵심 자료구조

**`queue.rs`** — FIFO 대기열 + 활성 사용자 관리

```rust
pub struct WaitingQueue {
    base_instant: Instant,                        // 타임스탬프 기준점
    active: HashMap<SessionId, ActiveSession>,    // 입장한 사용자
    waiting: VecDeque<QueueEntry>,                // FIFO 대기열
    waiting_index: HashMap<SessionId, usize>,     // O(1) 순번 조회
    generation: u64,                               // 인덱스 무효화용
    total_active_duration_secs: f64,              // ETA 계산용
    completed_sessions: u64,
}
```

- `last_seen`은 `AtomicU64`로 read lock에서도 갱신 가능 (핫패스 최적화)
- ETA 공식: `(position / max_active_users) * avg_active_duration`
- 7개 유닛 테스트 포함 (FIFO 순서, admit, touch, expire)

**`session.rs`** — HMAC-SHA256 서명 쿠키

- 토큰 포맷: `base64(session_id[16] | issued_at[8] | hmac[32])` = 56바이트
- 생성/검증 왕복 테스트, 변조 감지 테스트, 키 불일치 테스트 포함

**`state.rs`** — 공유 상태

```rust
pub struct AppState {
    pub config: RwLock<Config>,
    pub queue: Arc<dyn QueueBackend>,
    pub session_mgr: SessionManager,
    pub sse_tx: broadcast::Sender<()>,
    pub http_client: HttpClient,
    pub redis_pool: Option<Pool>,
}
```

### Phase 3: 리버스 프록시

**`proxy.rs`** — hyper 기반 HTTP 클라이언트

- `hyper_util::client::legacy::Client`로 커넥션 풀 재사용
- 요청 헤더에서 `Host` 제거, URI를 오리진으로 변환
- `hyper::body::Incoming` → `axum::body::Body` 변환

### Phase 4: Gate 미들웨어

**`middleware.rs`** — 모든 요청의 진입점

핵심 설계 결정: **`decide()` 함수에서 모든 lock을 `.await` 전에 해제**

- `parking_lot::RwLockReadGuard`는 `Send`가 아니라 `.await` 지점을 넘길 수 없음
- 해결: lock 접근을 동기 함수(`decide`)로 분리, 결과를 enum으로 반환 후 async 처리

리팩토링 후: `gate_check()` 단일 호출로 통합.

### Phase 5: 대기 페이지 + SSE

**`waiting.rs`** + **`templates/waiting.html`**

- `include_str!` + `minijinja`로 HTML 템플릿 내장 (빌드 도구 불필요)
- SSE: `/__wr/events` 엔드포인트, 쿠키 기반 인증 (HttpOnly 쿠키라 query param 대신 쿠키 직접 사용)
- `tokio::sync::broadcast` 채널로 큐 변경 알림 → 각 SSE 클라이언트가 자기 순번 재조회
- 입장 시 `{"action": "admit"}` 이벤트 → 브라우저 JS가 자동 리다이렉트

### Phase 6: Reaper (세션 만료)

**`reaper.rs`** — 백그라운드 태스크

- `tokio::time::interval`로 주기적 실행
- 만료 세션 정리 → 대기열에서 입장 → SSE 알림
- Redis 모드: `SET NX EX`로 리더 선출, 1대만 실행

### Phase 7: Admin API

**`admin.rs`** — 런타임 설정 변경

- `from_fn_with_state`로 X-Api-Key 인증 미들웨어
- 설정 변경, 통계 조회, 큐 flush 등

---

## 2. Redis 확장 구현

### Phase 1: Trait 추출 + MemoryBackend

**`backend.rs`** — `QueueBackend` trait + `MemoryBackend`

- `gate_check()`: 미들웨어의 전체 판단 로직을 하나로 통합
- `MemoryBackend`: 기존 `WaitingQueue`를 `parking_lot::RwLock`으로 래핑
- 기존 7개 테스트 모두 통과 (하위 호환)

### Phase 2: RedisBackend

**`redis_backend.rs`** — Lua 스크립트 기반 Redis 백엔드

- `deadpool-redis`로 커넥션 풀 관리 (기본 64개)
- 3개 Lua 스크립트로 원자적 연산:

**gate_check.lua** (핫패스, 1 round-trip):
```lua
-- 1. active 확인 → EXPIRE로 touch
-- 2. waiting 확인 → ZRANK로 위치 반환
-- 3. 입장 시도: HLEN < max → HSET + SET EX / ZADD
```

**reaper.lua** (만료 + 입장, atomic):
```lua
-- 1. HGETALL wr:active → EXISTS wr:active:{id}:ls 없는 세션 제거
-- 2. HINCRBYFLOAT/HINCRBY로 통계 갱신
-- 3. ZPOPMIN으로 대기열에서 입장
```

**flush.lua** (초기화):
```lua
-- HKEYS → DEL 각 presence key → DEL active, waiting, stats
```

### Phase 3: Pub/Sub 브릿지

**`pubsub.rs`** — Redis Pub/Sub → 로컬 SSE broadcast

- 전용 Redis 커넥션으로 `SUBSCRIBE wr:notify`
- 메시지 수신 시 `sse_tx.send(())` → 기존 SSE 핸들러 코드 변경 없음
- 연결 끊김 시 1초 후 자동 재연결

### Phase 4: Reaper 리더 선출

- `SET wr:reaper:lock {server_id} NX EX 10`
- 획득 실패 시 해당 tick skip
- 리더 서버 다운 시 10초 후 다른 서버가 자동 인계

---

## 3. 주요 기술적 결정

### axum 선택 (vs actix-web)

tower 미들웨어 생태계, `State` 추출기의 단순함, 내장 SSE 지원.

### HMAC-SHA256 쿠키 (vs JWT)

서버가 상태를 관리하므로 자체 포함 토큰 불필요. HMAC이 더 빠르고 작음.

### parking_lot::RwLock (vs tokio::RwLock)

in-memory 백엔드에서 lock 범위가 매우 짧아 (`touch()` 등) 동기 lock이 더 효율적. `.await` 넘기지 않는 설계로 Send 문제 회피.

### Lua 스크립트 (vs MULTI/EXEC)

gate_check의 조건 분기(`if active → touch, elif waiting → position, else → admit/enqueue`)를 파이프라인으로 표현 불가. Lua가 유일한 원자적 해법.

### 커넥션 풀 64개 (vs 256/512)

4대 서버 x 64 = 256 커넥션. Redis 단일 인스턴스 최적 범위. 512 이상은 오히려 Redis에 과부하.

---

## 4. 스케줄 기능 구현

### 개요

쿠폰 선착순 등 특정 시간에 오픈하는 이벤트를 위한 시간 기반 자동 제어.

### 신규 파일

**`src/scheduler.rs`**

- `Schedule` 구조체: `id`, `name`, `start_at`, `end_at`, `max_active_users`, `phase`
- `SchedulePhase` enum: `Pending` → `Active` → `Ended`
- `evaluate_schedules()`: 현재 시각 기준으로 phase 전환 판단, Active→Ended 전환 감지
- `spawn_scheduler()`: 1초마다 스케줄 체크, `config.enabled` 자동 변경. 스케줄 종료 시 자동 disable

### 수정 파일

| 파일 | 변경 |
|------|------|
| `state.rs` | `schedules: RwLock<Vec<Schedule>>` 필드 추가 |
| `admin.rs` | 스케줄 CRUD 엔드포인트 추가 |
| `main.rs` | `mod scheduler`, `spawn_scheduler()` 추가 |
| `Cargo.toml` | `chrono` 추가 |

### 스케줄 동작 메커니즘

`start_at` 도달 시:
- `config.enabled = true` 자동 전환
- `max_active_users`를 스케줄에 설정된 값으로 적용
- Gate 미들웨어와 Reaper가 정상 동작 → 대기열에서 순차 입장

`end_at` 도달 시:
- `config.enabled = false` 자동 전환 → 대기실 OFF, 트래픽 직통

### Admin API

```bash
# 스케줄 등록
curl -X POST -H "X-Api-Key: ..." -H "Content-Type: application/json" \
  -d '{
    "name": "쿠폰 이벤트",
    "start_at": "2026-04-15T10:00:00Z",
    "end_at": "2026-04-15T11:00:00Z",
    "max_active_users": 100
  }' http://localhost:8080/__wr/admin/schedules

# 스케줄 목록
curl -H "X-Api-Key: ..." http://localhost:8080/__wr/admin/schedules

# 스케줄 삭제
curl -X DELETE -H "X-Api-Key: ..." http://localhost:8080/__wr/admin/schedules/{id}
```

## 5. Admin SPA 구현

### 개요

React + TypeScript + Vite 기반 관리 대시보드. Waiting Room 서버의 Admin API를 호출하여 상태 모니터링 및 설정 변경.

### 기술 스택

- **React 19** + **TypeScript**
- **Vite** (빌드/개발 서버)
- **Tailwind CSS** (스타일링)

### 주요 페이지

| 페이지 | 경로 | 기능 |
|--------|------|------|
| Login | `/` | API Key 입력, localStorage 저장 |
| Dashboard | `/dashboard` | 실시간 큐 상태, 설정 변경, enable/disable 토글 |
| Schedules | `/schedules` | 스케줄 등록/삭제, phase 실시간 표시 |

### 주요 컴포넌트

| 컴포넌트 | 역할 |
|---------|------|
| `QueueVisualizer` | 활성 사용자/대기열 시각화 (애니메이션) |
| `StatusBadge` | Enabled/Disabled 상태 + Max Active + TTL 표시 |
| `Settings` | max_active_users, session_ttl 런타임 변경 |
| `Schedules` | 스케줄 목록 + 인라인 등록 폼 |

### 실행

```bash
cd admin
npm install
npm run dev       # 개발 서버 (http://localhost:5173)
npm run build     # 프로덕션 빌드 (dist/)
```

---

## 5. 의존성

| Crate | Version | 용도 |
|-------|---------|------|
| axum | 0.8 | 웹 프레임워크 |
| hyper + hyper-util | 1.x | 리버스 프록시 HTTP 클라이언트 |
| tokio | 1.x | 비동기 런타임 |
| parking_lot | 0.12 | 고성능 RwLock |
| hmac + sha2 | 0.12/0.10 | 쿠키 서명 |
| uuid | 1.x | 세션 ID |
| minijinja | 2.x | HTML 템플릿 |
| serde + toml | 1.x/0.8 | 설정 파싱 |
| tracing | 0.1 | 구조화 로깅 |
| async-trait | 0.1 | QueueBackend trait |
| redis | 0.27 | Redis 클라이언트 |
| deadpool-redis | 0.18 | Redis 커넥션 풀 |
| base64 | 0.22 | 토큰 인코딩 |
| rand | 0.9 | HMAC 시크릿 생성 |
| tokio-stream | 0.1 | SSE BroadcastStream |
| futures-util | 0.3 | StreamExt |
| chrono | 0.4 | 스케줄 시간 처리 |

---

## 5. 실행 방법

```bash
# 오리진 서버 (테스트용)
cargo run --example origin

# In-memory 모드
cargo run

# Redis 모드
WR_REDIS_URL="redis://127.0.0.1:6379" cargo run

# 멀티 서버
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8080" cargo run
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8081" cargo run
```
