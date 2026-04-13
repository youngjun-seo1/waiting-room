# Waiting Room - Cloudflare 비교 분석

## 1. 핵심 기능 비교

| 기능 | Cloudflare | Self-Hosted (본 프로젝트) |
|------|-----------|--------------------------|
| **대기열 방식** | FIFO / 랜덤 선택 가능 | FIFO |
| **세션 관리** | 쿠키 기반 | HMAC-SHA256 서명 쿠키 |
| **대기 페이지** | HTML/CSS 템플릿 편집기 | 완전한 커스텀 (HTML/JS) |
| **실시간 업데이트** | 주기적 폴링 | SSE 실시간 푸시 + 자동 입장 |
| **스케줄링** | 이벤트 기반 pre-queuing | start_at / end_at 자동 제어 + 종료 시 자동 disable |
| **분석/모니터링** | 내장 대시보드 | Admin SPA (React) |
| **API** | REST API (CRUD) | REST API (CRUD) |
| **수평 확장** | 글로벌 Edge (300+ PoP) | Redis 백엔드 멀티 서버 |

## 2. 아키텍처 차이

| | Cloudflare | Self-Hosted |
|--|-----------|-------------|
| **위치** | Edge (CDN 레벨) | Origin 앞단 (리버스 프록시) |
| **트래픽 흡수** | Origin에 트래픽 도달 전 Edge에서 차단 | 프록시 서버가 직접 트래픽 수용 |
| **인프라 부담** | 없음 (Cloudflare가 관리) | Redis + 프록시 서버 직접 관리 |
| **확장성** | 자동 (글로벌 Edge) | 수동 (서버 추가 + Redis Cluster) |

**Cloudflare (Edge-side)**

```
Client → [Cloudflare Edge PoP] → 입장 가능? → Origin
                   ↓ No
           [Queue Page at Edge]
```

트래픽이 Cloudflare Edge에서 차단되므로 Origin은 폭주 트래픽을 아예 보지 않음.

**Self-Hosted (Origin-side)**

```
Client → [Waiting Room Proxy] → 입장 가능? → Origin
                  ↓ No
          [Queue Page at Proxy]
```

프록시 서버가 직접 폭주 트래픽을 받아야 함. Redis 멀티 서버로 수평 확장 가능.

## 3. 비용 비교

| | Cloudflare | Self-Hosted |
|--|-----------|-------------|
| **최소 요건** | Business Plan (~$200/월) | 서버 인프라 비용만 |
| **Waiting Room 수** | Business: 1개 / Enterprise: 무제한 | 제한 없음 |
| **대규모** | Enterprise (커스텀 가격) | Redis + 서버 비용 비례 증가 |
| **소규모** | $200/월 이상 | 서버 1대 + in-memory (거의 무료) |

## 4. Cloudflare 장점

- **Edge에서 트래픽 흡수**: Origin 서버가 폭주 트래픽을 전혀 받지 않음
- **글로벌 분산**: 300+ PoP에서 대기 페이지를 서빙, 지연 시간 최소화
- **관리형 서비스**: 운영 부담 없음 (스케일링, 모니터링, 장애 대응)
- **내장 분석**: 대기열 깊이, 대기 시간, 활성 사용자 대시보드 기본 제공

## 5. Self-Hosted 장점

- **벤더 종속 없음**: 어떤 인프라에서든 동작, Cloudflare DNS/프록시 불필요
- **완전한 커스텀**: 대기열 로직, UX, 페어니스 알고리즘 자유 제어
- **SSE 실시간 푸시**: Cloudflare는 폴링 방식이지만, SSE로 즉시 순번 갱신 + 자동 입장 리다이렉트
- **비용 효율**: 소규모에서 훨씬 저렴 (서버 1대 + in-memory로 충분)
- **스케줄 자동 disable**: 종료 시간에 자동으로 대기실 OFF
- **투명한 내부**: 오픈소스, 동작 원리 완전 파악 가능

## 6. 핵심 트레이드오프

> Cloudflare는 Edge에서 폭주 트래픽을 흡수하므로 Origin이 안전하지만,
> Self-Hosted는 **프록시 서버 자체가 폭주를 견뎌야** 한다.

다만 벤치마크 결과:

| 구성 | 동접 | 에러 | 서버당 리소스 |
|------|------|------|-------------|
| 4서버 (Redis) | 10,000 | 0건 | CPU 30%, RSS 31MB |
| 4서버 (Redis) | 50,000 | 있음 | CPU 45%, RSS 31MB |
| 1서버 (in-memory) | 1,000 | 0건 | 6,578 req/s |

10,000 동접까지는 에러 0건으로 충분히 실용적.

## 7. 선택 가이드

| 상황 | 추천 |
|------|------|
| 글로벌 대규모 트래픽 (수만~수십만 동접) | Cloudflare |
| 이미 Cloudflare를 사용 중 | Cloudflare |
| 운영 부담을 최소화하고 싶을 때 | Cloudflare |
| 벤더 종속을 피하고 싶을 때 | Self-Hosted |
| 대기열 로직을 완전히 커스텀해야 할 때 | Self-Hosted |
| 소규모 이벤트 (수천 동접 이하) | Self-Hosted (비용 효율) |
| 내부 시스템/사내 서비스 | Self-Hosted |

## 8. 참고 자료

- [Cloudflare Waiting Room - About](https://developers.cloudflare.com/waiting-room/about/)
- [Cloudflare Waiting Room - Configuration Settings](https://developers.cloudflare.com/waiting-room/reference/configuration-settings/)
- [Cloudflare Waiting Room - FAQ / Troubleshooting](https://developers.cloudflare.com/waiting-room/troubleshooting/)

> 참고: Cloudflare Waiting Room은 대기 페이지에서 20초마다 HTTP refresh 헤더로 자동 새로고침하는 방식을 사용한다. (SSE/WebSocket 아님)
