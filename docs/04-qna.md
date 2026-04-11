# Waiting Room - Q&A

## Q1. cargo run 하면 waiting room, origin 서버가 같이 구동되는거야?

아니요, 별도입니다.

- `cargo run` → Waiting Room 프록시 (8080 포트)만 기동
- Origin 서버 (3000 포트)는 따로 실행해야 함
- Waiting Room은 리버스 프록시로, origin이 없으면 502 Bad Gateway 발생

```bash
# 터미널 1: Origin
cargo run --example origin

# 터미널 2: Waiting Room
cargo run
```

---

## Q2. 지금 로컬에서 최대 몇 명까지 동접할 수 있어?

테스트 환경 (14코어, 48GB RAM) 기준:

| 동접 | 결과 |
|------|------|
| 1,000 | 3,448 req/s, 에러 0 |
| 5,000 | 에러 0 (Redis 4서버) |
| 10,000 | 에러 0 (Redis 4서버) |
| 20,000+ | 클라이언트 TCP 포트 고갈로 에러 발생 (서버는 정상) |

서버 자체는 100K 요청에서도 다운되지 않으며, 메모리 337MB, CPU 45% 수준.

---

## Q3. 지금 멀티 서버로 확장 가능한 구조인가?

초기 in-memory 버전은 불가능했으나, Redis 백엔드 추가 후 가능.

- `redis_url` 미설정 → in-memory (단일 서버)
- `redis_url` 설정 → Redis (멀티 서버)

```bash
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8080" cargo run
WR_REDIS_URL="redis://127.0.0.1:6379" WR_LISTEN_ADDR="0.0.0.0:8081" cargo run
```

---

## Q4. Lua 스크립트는 어떤 용도인거지?

Redis에서 **여러 명령을 하나의 원자적 연산으로 묶기 위해** 사용.

3개 Lua 스크립트:

| 스크립트 | 용도 |
|---------|------|
| gate_check | 미들웨어 핫패스 (active 확인→touch / waiting→위치 / 입장 또는 대기열) |
| reaper | 만료 세션 정리 + 대기열에서 입장 |
| flush | 큐 전체 초기화 |

별도 lock 없이 원자성 보장 — 멀티 서버에서 max_active 초과 불가.

---

## Q5. 그럼 WR 서버에서 Lua 스크립트를 실행하는거지?

아닙니다. **Redis 서버가** 실행합니다.

```
WR 서버 → EVAL "lua 코드" → Redis 서버 (Lua 실행) → 결과 반환
```

Redis는 싱글 스레드이므로 Lua 실행 중 다른 명령이 끼어들 수 없음. 이것이 원자성의 핵심.

---

## Q6. Reaper는 뭐하는 녀석이야?

**만료된 세션을 정리하고, 대기열에서 다음 사용자를 입장시키는 백그라운드 태스크.**

```
매 1초마다:
1. active 세션 스캔 → TTL 지난 세션 제거
2. 빈 슬롯만큼 대기열에서 다음 사용자 입장
3. SSE 알림 → 대기 페이지 순번 갱신 / 자동 리다이렉트
```

멀티 서버에서는 `SET NX EX`로 리더 1대만 선출하여 실행. 리더 다운 시 10초 후 자동 인계.

---

## Q7. SSE는 클라이언트와 서버 간 웹소켓으로 연결하고 서버가 데이터를 쏴주는거야?

웹소켓이 아닙니다. SSE(Server-Sent Events)는 별도 프로토콜입니다.

| | SSE | WebSocket |
|--|-----|-----------|
| 방향 | 서버 → 클라이언트 (단방향) | 양방향 |
| 프로토콜 | 일반 HTTP | HTTP 업그레이드 후 별도 프로토콜 |
| 재연결 | 브라우저 자동 | 직접 구현 필요 |

대기 페이지에서는 서버가 순번을 일방적으로 푸시하면 되므로 SSE가 적합. 더 단순하고 프록시/CDN 호환성도 좋음.

---

## Q8. 중간에 LB가 있어도 문제될 소지는 없어?

대부분 문제없지만 주의할 점 2가지:

**1. SSE 타임아웃**: LB가 유휴 연결을 끊을 수 있음

```nginx
location /__wr/events {
    proxy_read_timeout 3600s;
    proxy_buffering off;
}
```

**2. Sticky session**: 불필요. 쿠키가 stateless(HMAC)이고 큐 상태는 Redis에 있으므로 라운드로빈으로 충분.

---

## Q9. 대기 페이지는 HTML 파일을 렌더링하고 있어?

네. `src/templates/waiting.html`을 **빌드 시점에 바이너리에 내장**(`include_str!`)하고, 요청 시 `minijinja`로 변수를 치환해서 반환.

```rust
static TEMPLATE_SRC: &str = include_str!("templates/waiting.html");

tmpl.render(context! { position => 50, eta_display => "약 5분", ... })
```

바이너리 하나에 모든 것이 포함 — 별도 파일 서빙이나 CDN 불필요.

---

## Q10. 다른 HTML로 변경하려면 어떻게 해야해?

현재는 `src/templates/waiting.html` 수정 후 재컴파일 필요.

런타임에 외부 HTML 로드 기능을 추가하면 재컴파일 없이 파일만 교체 가능:

```toml
[branding]
custom_template_path = "/path/to/my-waiting.html"
```

---

## Q11. Origin 서버에서 waiting room을 사용하기 위해서 추가로 구현해야 할게 있어?

**없습니다.** Origin은 아무것도 변경할 필요 없음.

Waiting Room은 리버스 프록시로 Origin 앞에 투명하게 위치:

```
[변경 전]  Client → Origin
[변경 후]  Client → Waiting Room → Origin
```

설정에 Origin 주소만 넣으면 끝:

```toml
origin_url = "http://my-origin:3000"
```

---

## Q12. 쿠폰 선착순처럼 특정 시간에 오픈해야 한다면 어떻게 운영해야 하지?

스케줄 기능으로 자동화. Admin SPA 또는 Admin API로 등록:

**Admin SPA:**
1. Schedules 페이지에서 Name, Start At, End At, Max Active Users 입력
2. Create Schedule 버튼 클릭
3. 스케줄이 자동으로 start_at에 대기실 ON, end_at에 대기실 OFF

**Admin API:**

```bash
# 스케줄 등록
curl -X POST -H "X-Api-Key: ..." -H "Content-Type: application/json" \
  -d '{
    "name": "쿠폰 이벤트",
    "start_at": "2026-04-15T10:00:00Z",
    "end_at": "2026-04-15T11:00:00Z",
    "max_active_users": 100
  }' http://localhost:8080/__wr/admin/schedules

# 스케줄 목록 조회
curl -H "X-Api-Key: ..." http://localhost:8080/__wr/admin/schedules

# 스케줄 삭제
curl -X DELETE -H "X-Api-Key: ..." http://localhost:8080/__wr/admin/schedules/{id}
```

**2단계 라이프사이클:**

```
[pending]                [active]              [ended]
 대기실 OFF    start_at→  대기실 ON      end_at→  대기실 자동 OFF
 Origin 직접 접근         순차 입장 시작           트래픽 직통
```
