# Waiting Room - 테스트 문서

## 1. 유닛 테스트

### queue.rs (7개 테스트)

```bash
cargo test
```

| 테스트 | 검증 내용 |
|--------|----------|
| `test_fifo_ordering` | 5명 enqueue → 순번 1~5 확인 → 2명 admit → 나머지 순번 갱신 |
| `test_admit_direct` | 직접 admit → is_active 확인, active_count 확인 |
| `test_touch_updates_last_seen` | admit → 10ms 대기 → touch → last_seen 증가 확인 |

### session.rs (4개 테스트)

| 테스트 | 검증 내용 |
|--------|----------|
| `test_token_roundtrip` | 토큰 생성 → 검증 → 동일 SessionId 반환 |
| `test_tampered_token_rejected` | 토큰 1바이트 변조 → 검증 실패 |
| `test_wrong_key_rejected` | 다른 키로 검증 → 실패 |
| `test_invalid_token` | 빈 문자열, 잘못된 형식 → None 반환 |

---

## 2. 기능 테스트

### 2.1 기본 동작 (In-memory)

```bash
# 서버 기동
cargo run --example origin &
cargo run &

# 첫 번째 사용자: 입장
curl -s -c /tmp/u1.txt http://localhost:8080/ | grep '<title>'
# → <title>티켓 구매</title>

# 두 번째 사용자: 대기 (max_active_users=1일 때)
curl -s -c /tmp/u2.txt http://localhost:8080/ | grep '<title>'
# → <title>Please wait...</title>

# 상태 확인
curl -s http://localhost:8080/__wr/status
# → {"active_users":1,"enabled":true,"queue_length":1}
```

### 2.2 자동 입장 테스트

```bash
# TTL을 5초로 설정
curl -X PUT -H "X-Api-Key: change-me-in-production" \
  -H "Content-Type: application/json" \
  -d '{"session_ttl_secs": 5}' \
  http://localhost:8080/__wr/admin/config

# keep-alive 중단 → 5~10초 후 대기열 사용자 자동 입장
# 브라우저에서 확인: 대기 페이지 → 자동 리다이렉트 → 티켓 구매 페이지
```

### 2.3 SSE 실시간 업데이트

- 대기 페이지 접속 → Network 탭에서 `/__wr/events` SSE 연결 확인
- 순번, ETA가 실시간으로 갱신되는지 확인
- 입장 시 `{"action":"admit"}` 이벤트 → 자동 리다이렉트

### 2.4 Redis 모드 테스트

```bash
redis-server --daemonize yes
WR_REDIS_URL="redis://127.0.0.1:6379" cargo run &

# 동일한 기능 테스트 + Redis 키 확인
redis-cli keys 'wr:*'
# → wr:active, wr:active:{id}:ls, wr:waiting, wr:reaper:lock
```

### 2.5 멀티 서버 테스트

```bash
# 서버 2대 기동
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8080" cargo run &
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8081" cargo run &

# 서버A에서 슬롯 점유
curl -c /tmp/occ.txt http://localhost:8080/

# 서버B에서 접속 → 대기열
curl http://localhost:8081/ | grep '<title>'
# → <title>Please wait...</title>

# 양쪽 서버 상태 동일 확인
curl -s http://localhost:8080/__wr/status
curl -s http://localhost:8081/__wr/status
# → 동일한 active_users, queue_length
```

---

## 3. 부하 테스트 (Rust)

SSE 기반 부하 테스트 도구. tokio 비동기 런타임으로 수만 개의 동시 연결을 단일 프로세스에서 처리.

### 3.0 사전 준비 (macOS)

macOS 기본 설정으로는 동시 연결 수가 제한됩니다. 테스트 전 아래 설정이 필요합니다.

```bash
# TCP listen backlog 상향 (기본값 128 → 8192)
sudo sysctl -w kern.ipc.somaxconn=8192

# 파일 디스크립터 제한 상향 (기본값 256 → 65536)
sudo launchctl limit maxfiles 65536 200000
```

> 두 명령 모두 재부팅 시 초기화됩니다. 영구 적용은 `/etc/sysctl.conf`와 `/Library/LaunchDaemons/limit.maxfiles.plist`를 참고.

설정 변경 후 **서버와 터미널을 모두 재시작**해야 적용됩니다. 확인:

```bash
sysctl kern.ipc.somaxconn     # → 8192
launchctl limit maxfiles       # → 65536  unlimited
ulimit -n                      # → 65536 (터미널 재시작 필요)
```

### 3.1 시나리오 부하 테스트 (`load_test`)

실제 브라우저 동작을 시뮬레이션. 각 유저가 `GET /` → 쿠키 획득 → SSE 연결 → admit 이벤트 대기 → 입장 완료.

```bash
# 기본 (1000명, 동시 500)
./tests/run_load_test.sh

# 옵션 지정
./tests/run_load_test.sh --total 10000 --concurrency 2000

# 전체 옵션
./tests/run_load_test.sh --help
```

| 옵션 | 기본값 | 설명 |
|------|--------|------|
| `--total` | 1000 | 총 시뮬레이션 유저 수 |
| `--concurrency` | 500 | 최대 동시 실행 태스크 |
| `--sse-timeout` | 120 | SSE admit 대기 타임아웃 (초) |
| `--url` | http://localhost:8080 | 서버 URL |
| `--retries` | 3 | 세션 만료 시 재시도 횟수 |

출력 예시:

```
=== Waiting Room SSE Load Test ===
Total users: 1000
Concurrency: 500

  [100/1000] direct:100 sse:0 timeout:0 error:0
  [500/1000] direct:100 sse:400 timeout:0 error:0
  [1000/1000] direct:100 sse:900 timeout:0 error:0

[Results] 45.2s elapsed
  Admitted (direct):  100    ← max_active_users만큼 바로 입장 (302)
  Admitted (SSE):     900    ← 대기 후 SSE admit으로 입장
  Timeout:            0
  Errors:             0
```

### 3.2 동시 연결 수 테스트 (`conn_test`)

서버가 실제로 몇 개의 SSE 연결을 동시에 유지할 수 있는지 측정. admit 여부와 무관하게 연결만 열고 hold 시간 동안 유지.

```bash
# 기본 (1000개, 초당 200개 생성, 30초 유지)
./tests/run_conn_test.sh

# 대규모 테스트
./tests/run_conn_test.sh --target 5000 --rate 500 --hold 60

# 10000개 연결 테스트
./tests/run_conn_test.sh --target 10000 --rate 1000 --hold 30
```

| 옵션 | 기본값 | 설명 |
|------|--------|------|
| `--target` | 1000 | 목표 동시 연결 수 |
| `--rate` | 200 | 초당 연결 생성 속도 |
| `--hold` | 30 | 연결 유지 시간 (초) |
| `--url` | http://localhost:8080 | 서버 URL |

출력 예시:

```
=== SSE Max Connection Test ===
Target:    5000 connections
Rate:      500/sec
Hold:      60s

  active:  1000  peak:  1000  opened:  1000  err:0/0  dropped:0
  active:  2000  peak:  2000  opened:  2000  err:0/0  dropped:0
  active:  5000  peak:  5000  opened:  5000  err:0/0  dropped:0

=== Results (76.5s) ===
  Peak concurrent SSE:  5000
  Total opened:         5000
  Connect errors:       0
  SSE errors:           0
  Dropped during hold:  0

  PASS: reached target 5000 connections
```

### 3.3 두 테스트의 차이

| | load_test | conn_test |
|---|---|---|
| 목적 | 유저 시나리오 (입장까지) | 순수 동시 연결 수 측정 |
| SSE 동작 | admit 받으면 연결 종료 | hold 시간 동안 유지 |
| 핵심 지표 | admitted / timeout / error | **peak concurrent connections** |
| 연결 생성 | 전원 동시 (세마포어 제어) | rate 제어 (초당 N개) |

### 3.4 트러블슈팅

| 증상 | 원인 | 해결 |
|------|------|------|
| peak이 ~245에서 멈춤 | `ulimit -n` 256 (macOS 기본) | `ulimit -n 65536` 또는 `run_*.sh` 사용 |
| "error sending request" 다수 | `kern.ipc.somaxconn` 128 | `sudo sysctl -w kern.ipc.somaxconn=8192` |
| "Waiting room is disabled" | 활성 스케줄 없음 | Admin에서 스케줄 생성 |
| 서버 변경 후에도 동일 증상 | 서버 미재시작 (이전 프로세스 FD 제한 유지) | 서버 재시작 |

---

## 4. 성능 테스트

### 4.1 단일 서버 In-memory vs Redis

```bash
# In-memory
cargo run --release &
cargo run --release --example bench

# Redis
WR_REDIS_URL="redis://..." cargo run --release &
cargo run --release --example bench
```

**결과:**

| 동접 | In-Memory | Redis |
|------|-----------|-------|
| 1,000 | 6,578 req/s, 0 err | 3,663 req/s, 0 err |
| 5,000 | 630 req/s, 2,225 err | 840 req/s, 0 err |
| 10,000 | 1,248 req/s, 3,553 err | 1,247 req/s, 3,104 err |
| 50,000 | 2,095 req/s | 3,197 req/s |

**분석:**
- 1,000명 이하: In-memory가 ~2배 빠름 (네트워크 오버헤드 없음)
- 5,000명: Redis가 에러 0으로 더 안정적 (Lua 원자성)
- 50,000명: Redis가 오히려 더 빠름 (RwLock 경합 vs Redis 직렬화)

### 4.2 멀티 서버 스케일링 (Redis, 4서버)

```bash
# 4대 서버 기동
for port in 8080 8081 8082 8083; do
  WR_REDIS_URL="redis://..." WR_LISTEN_ADDR="0.0.0.0:$port" cargo run --release &
done

cargo run --release --example bench_multi
```

**처리량 결과:**

| 동접 | 시간 | 성공 | 에러 | req/s |
|------|------|------|------|-------|
| 1,000 | 307ms | 1,000 | 0 | 3,257 |
| 5,000 | 1.3s | 5,000 | 0 | 3,872 |
| 10,000 | 2.7s | 10,000 | 0 | 3,734 |
| 20,000 | 4.4s | 6,962 | 13,038 | 4,561 |
| 50,000 | 10.5s | 10,276 | 39,724 | 4,777 |
| 100,000 | 22.7s | 35,649 | 64,351 | 4,405 |

**서버 리소스 사용량:**

| 동접 | 서버당 Avg CPU | 서버당 Peak CPU | 서버당 Peak RSS | 4대 합계 RSS |
|------|---------------|----------------|----------------|-------------|
| 1,000 | 4.6% | 9.2% | 18~20MB | 76MB |
| 5,000 | 24.8% | 42.4% | 28~31MB | 118MB |
| 10,000 | 29.3% | 40.0% | 29~31MB | 120MB |
| 100,000 | 12.9% | 45.6% | 29~31MB | 120MB |

**Redis 리소스:**

| 동접 | Redis ops/s | Redis mem |
|------|-------------|-----------|
| 1,000 | 8,102 | 37MB |
| 5,000 | 40,366 | 37MB |
| 10,000 | 48,586 | 37MB |

### 4.3 스케일링 비교 (1서버 vs 2서버 vs 4서버)

| 동접 | 1서버 에러 | 2서버 에러 | 4서버 에러 |
|------|----------|----------|----------|
| 1,000 | 0 | 0 | **0** |
| 5,000 | 0 | 378 | **0** |
| 10,000 | 3,742 | 447 | **0** |

**10,000명 동접에서 4서버는 에러 0건** — 서버 추가로 에러가 완전히 제거됨.

---

## 5. 주요 성능 분석

### 5.1 병목 지점

- **10,000명 이하**: 서버 CPU 30%, 메모리 30MB — 여유 충분
- **20,000명+**: 로컬 TCP 포트 고갈이 주 에러 원인 (서버가 아닌 벤치 클라이언트 한계)
- **Redis**: 피크 48,586 ops/s — 단일 인스턴스 한계(~100K ops/s)의 절반

### 5.2 최적화 포인트

| 항목 | 현재 값 | 근거 |
|------|--------|------|
| Redis 풀 크기 | 64/서버 | 4대 x 64 = 256 커넥션. 512+는 Redis 과부하 |
| Reaper 간격 | 1초 | 빠른 입장 전환 vs Redis 부하 균형 |
| SSE keep-alive | axum 기본값 | 15초 간격 ping |
| 벤치 배치 크기 | 2,000 | fd 고갈 방지, 안정적 측정 |

### 5.3 프로덕션 확장 시 기대 성능

- **서버 N대**: 처리량 선형 증가 (현재 병목은 클라이언트 쪽)
- **Redis Cluster**: ops/s 한계 제거
- **분산 클라이언트**: 20,000명+ TCP 포트 문제 해결
- **서버당 리소스**: CPU 30%, RSS 31MB로 매우 효율적 — 저사양 서버에서도 운용 가능

---

## 6. 상태 동기화 검증

모든 벤치마크에서 멀티 서버 상태 일관성을 검증:

```
S0: a=10 q=9990  S1: a=10 q=9990  S2: a=10 q=9990  S3: a=10 q=9990
```

- **active_users**: 모든 서버에서 항상 동일, max_active_users 초과 0건
- **queue_length**: 모든 서버에서 항상 동일
- **STATE MISMATCH**: 전체 테스트에서 0건

---

## 7. 테스트 환경

| 항목 | 값 |
|------|-----|
| CPU | Apple Silicon, 14코어 |
| RAM | 48GB |
| OS | macOS (Darwin 25.2.0) |
| Rust | 1.94.1 (release 빌드) |
| Redis | 로컬 단일 인스턴스 |
| fd limit | unlimited |
| 프로세스 제한 | 8,000 |

---

## 8. 스케줄 기능 테스트

### 8.1 테스트 시나리오

스케줄을 등록하여 start_at → end_at 라이프사이클 검증.

```bash
# 대기실 비활성화 (스케줄이 제어)
curl -X POST -H "X-Api-Key: ..." http://localhost:8080/__wr/admin/disable

# 스케줄 등록
curl -X POST -H "X-Api-Key: ..." -H "Content-Type: application/json" \
  -d '{
    "name": "쿠폰 이벤트",
    "start_at": "2026-04-10T06:30:26Z",
    "end_at": "2026-04-10T06:32:16Z",
    "max_active_users": 100
  }' http://localhost:8080/__wr/admin/schedules
```

### 8.2 Phase별 검증 결과

| Phase | 시점 | 상태 | 사용자 접속 결과 | 검증 |
|-------|------|------|-----------------|------|
| `pending` | 등록 직후 | enabled=false | "이벤트 참여 시간이 아닙니다" 페이지 | OK |
| `active` | start_at 도달 | enabled=true | 대기실 ON, 순차 입장 시작 | OK |
| `ended` | end_at 도달 | enabled=false | 대기열 flush, SSE "closed" 이벤트, 종료 안내 | OK |

### 8.3 Active phase 상세

```
start_at 도달 시:
- config.enabled = true 자동 전환
- phase: "active"
- 대기열에서 max_active_users만큼 순차 입장
- 입장된 사용자: Origin 페이지 정상 접근
- TTL 만료 → 다음 사용자 자동 입장
```

### 8.4 Ended phase 상세

```
end_at 도달 시:
- config.enabled = false 자동 전환 (대기실 OFF)
- phase: "ended"
- 대기열 flush (모든 대기 세션 제거)
- SSE "closed" 이벤트 전송 → 대기 페이지가 "이벤트가 종료되었습니다" 안내로 전환
- 새 접속 시 "이벤트 참여 시간이 아닙니다" 페이지 표시
```

### 8.5 운영 흐름 예시

```
10:00 start_at → 대기실 ON, 대기열에서 순차 입장 시작
11:00 end_at   → 이벤트 종료, 대기실 자동 OFF
```
