# 시스템 설계 리뷰

## 1. 전체 아키텍처

Cloudflare Waiting Room 스타일의 리버스 프록시 기반 대기열 시스템.
Rust(axum/tokio) + 선택적 Redis 백엔드.

```
Client → Gate Middleware → [Active?] → Reverse Proxy → Origin
                        → [Queue]  → SSE Waiting Page → (promoted) → Redirect → Origin
```

- 모든 요청은 Gate Middleware를 거쳐 세션 상태에 따라 분기
- Active 세션은 origin으로 프록시, 대기 중인 세션은 SSE 기반 대기 페이지로 안내
- 대기열에서 빠져나오면 클라이언트가 자동 리다이렉트

---

## 2. 핵심 설계 포인트

### 2.1 Backend 추상화 (Trait Object)

`QueueBackend` trait으로 Memory / Redis 백엔드를 교체 가능하게 설계.

```rust
pub trait QueueBackend: Send + Sync + 'static {
    async fn gate_check(...) -> GateResult;
    async fn get_position(...) -> Option<QueuePosition>;
    async fn is_active(...) -> bool;
    async fn stats(...) -> QueueStats;
    async fn flush(...);
    async fn reaper_cycle(...) -> (usize, usize);
}
```

- 단일 서버: Memory 백엔드 (기본값)
- 수평 확장: Redis 백엔드 (config 변경만으로 전환)
- `GateResult` enum으로 4가지 상태 표현: `Active`, `Waiting`, `Admitted`, `Enqueued`

### 2.2 Redis 원자성 — Lua Script

gate_check, reaper_cycle, flush를 모두 Lua 스크립트로 원자적 실행.

| Redis 자료구조 | 용도 |
|---------------|------|
| `wr:active` (HASH) | Active 세션 목록 (session_id → admitted_ms) |
| `wr:active:{id}:ls` (KEY + EX) | Active 세션의 TTL marker |
| `wr:waiting` (ZSET) | 대기열 (score = join 시간, FIFO 보장) |
| `wr:stats` (HASH) | 누적 통계 (total_duration_ms, completed_sessions 등) |

**TTL marker 이중 구조**: HASH만으로는 필드별 자동 만료가 불가능하므로, 별도 key에 EX를 설정하여 세션 만료를 감지한다. Lua 스크립트가 `EXISTS wr:active:{id}:ls`로 생존 여부를 확인하고, 없으면 HASH에서도 제거.

### 2.3 분산 조율 패턴

멀티 인스턴스 환경에서의 조율 전략.

| 문제 | 해결 방식 |
|------|----------|
| Reaper 중복 실행 방지 | `wr:reaper:lock` SET NX (leader election) |
| HMAC secret 공유 | Redis SET NX (first-writer-wins) |
| 스케줄 시작 시 flush 중복 방지 | `wr:flushed:{id}` SETNX + 24h TTL (exactly-once) |
| 스케줄 종료 시 cleanup 중복 방지 | `wr:ended:{id}` SETNX + 24h TTL (exactly-once) |
| 인스턴스 간 상태 동기화 | Redis Pub/Sub `wr:notify` 채널 |

### 2.4 세션 토큰 설계

HMAC-SHA256 서명 기반 stateless 토큰.

```
Token = base64url(uuid[16] + issued_at[8] + hmac_sha256[32])
```

- 총 56바이트 → base64url 인코딩
- 서버 간 동일 secret 공유로 어느 인스턴스에서든 검증 가능
- 쿠키 설정: HttpOnly, SameSite=Lax, 24시간 max-age
- 토큰 자체에 expiry 없음 — TTL은 Redis marker 또는 in-memory Instant 기반으로 별도 관리

### 2.5 실시간 통신 (SSE)

`tokio::sync::broadcast` 채널 기반 실시간 업데이트.

```json
{
  "position": 5,
  "total_waiting": 42,
  "eta_seconds": 125.5,
  "progress_pct": 88.1,
  "action": "admit" | "closed" | null,
  "redirect_url": "https://..."
}
```

- 채널 버퍼 1024개
- 큐 변경 시 broadcast → 각 SSE 스트림이 현재 position 조회 후 전송
- `action: "admit"` 수신 시 클라이언트가 origin으로 자동 리다이렉트
- `action: "closed"` 수신 시 종료 안내 표시

### 2.6 스케줄 기반 자동 제어

1초 tick loop으로 스케줄 phase를 평가.

```
Pending → Active → Ended
         (start_at)  (end_at)
```

- **Active 전환 시**: 대기실 ON + config override (origin_url, max_active_users 등) + 큐 flush
- **Ended 전환 시**: 대기열 flush + "closed" SSE 이벤트 + 대기실 OFF
- **아카이브**: 종료 후 retention_secs(기본 24시간) 경과 시 archives로 이동
- **Multi-instance**: SETNX로 exactly-once 보장 (flush, cleanup)

### 2.7 ETA 추정 알고리즘

완료된 세션의 실제 체류 시간 기반 추정.

```
avg_duration = total_active_duration / completed_sessions  (기본값: 300초)
eta = (position / max(active_count, 1)) × avg_duration
```

- 초기에는 기본값(300초) 사용
- 시간이 지날수록 실제 데이터가 쌓여 정확도 향상
- 세션 만료 시 `total_active_duration`에 체류 시간 누적

### 2.8 Reaper (세션 만료 + 입장 승격)

백그라운드 task로 주기적(기본 1초) 실행.

1. TTL 초과한 active 세션 만료 처리
2. 빈 슬롯만큼 대기열에서 FIFO 순서로 승격
3. SSE broadcast로 전체 대기자에게 알림

Redis 모드에서는 leader election(`SET NX`)으로 한 인스턴스만 실행.

---

## 3. In-Memory 백엔드 상세

### 자료구조

```rust
pub struct WaitingQueue {
    active: HashMap<SessionId, ActiveSession>,     // O(1) 조회
    waiting: VecDeque<QueueEntry>,                 // FIFO
    waiting_index: HashMap<SessionId, usize>,      // O(1) position 조회
    total_active_duration_secs: f64,               // ETA 계산용
    completed_sessions: u64,                       // ETA 계산용
}
```

### 동시성 전략

- `RwLock<WaitingQueue>`: gate_check 시 read lock으로 active/waiting 확인, write lock으로 admit/enqueue
- `AtomicU64`: `last_seen` 갱신 — lock 없이 touch 가능
- `AtomicBool(Relaxed)`: enabled 플래그 — 인스턴스 간 최종 일관성 허용

---

## 4. 컴포넌트 의존 관계

```
main.rs
├── config.rs          (설정 로드)
├── state.rs           (AppState 생성, HMAC 동기화)
│   ├── backend.rs     (MemoryBackend)
│   ├── redis_backend.rs (RedisBackend)
│   ├── queue.rs       (WaitingQueue 자료구조)
│   └── session.rs     (SessionManager)
├── middleware.rs       (Gate 핸들러)
│   └── waiting.rs     (대기 페이지 + SSE)
├── proxy.rs           (리버스 프록시)
├── admin.rs           (Admin API)
│   ├── schedule_store.rs (스케줄 CRUD)
│   └── archive_store.rs  (아카이브 저장)
├── reaper.rs          (세션 만료 + 승격)
├── scheduler.rs       (스케줄 자동 제어)
└── pubsub.rs          (Redis Pub/Sub 동기화)
```

---

## 5. 리뷰 논의 포인트

### 5.1 Eventual Consistency

`enabled` 상태가 `AtomicBool(Relaxed)` + Pub/Sub 기반.

- 인스턴스 간 짧은 불일치 구간이 존재할 수 있음
- 대기열 시스템 특성상 수 ms 불일치는 허용 가능한 trade-off
- Pub/Sub 유실 시 다음 scheduler tick(1초)에서 보정됨

### 5.2 Pub/Sub 메시지 유실

Redis Pub/Sub은 fire-and-forget 모델.

- 연결 끊김 시 메시지 유실 가능
- 보완: startup 시 `load_enabled_from_redis()`, scheduler 1초 tick에서 주기적 동기화
- 큐 데이터 자체는 Redis에 있으므로 유실 영향은 `enabled` 플래그 동기화에 한정

### 5.3 Reaper Lock TTL

Lock TTL = `interval + 1초`.

- Lua 스크립트 실행이 예상보다 오래 걸리면 lock이 만료되어 다중 실행 가능
- 실제 Lua 스크립트는 ms 단위로 완료되므로 현실적 위험은 낮음
- 최악의 경우 중복 reaper가 돌아도 Lua의 원자성이 데이터 정합성을 보장

### 5.4 세션 토큰에 만료 시간 없음

토큰 자체에 expiry가 포함되지 않음.

- 세션 유효성은 Redis TTL marker(또는 in-memory Instant)에 전적으로 의존
- 토큰이 탈취되어도 TTL marker가 만료되면 무효 → 실질적 위험 제한적
- 필요 시 토큰에 `exp` 필드를 추가하여 이중 검증 가능

### 5.5 Origin 장애 대응

현재 proxy에서는 단순 502 반환만 수행.

- Circuit breaker, 재시도, 헬스체크 없음
- 의도적 단순화: 대기실의 역할은 트래픽 조절이지 origin 가용성 보장이 아님
- 필요 시 origin 헬스체크 + 자동 대기실 활성화 연동 고려 가능

### 5.6 스케줄 동시성

스케줄 생성 시 overlap 검증이 존재하지만, 동시에 하나만 active인 모델.

- 복수 이벤트 동시 운영이 필요하면 설계 확장 필요
- 현재 config override 방식(단일 origin_url 등)이 단일 스케줄 전제

### 5.7 Memory 모드 한계

서버 재시작 시 모든 세션/큐 유실.

- 개발/테스트 용도로는 충분
- 프로덕션에서는 Redis 모드 필수
