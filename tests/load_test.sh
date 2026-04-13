#!/bin/bash
# 1000명 동시 접속 부하 테스트
# max_active=10, session_ttl=1s, reaper_interval=1s
# → 10명씩 ~1초 간격으로 순차 입장

set -uo pipefail

TOTAL="${1:-1000}"
WR_URL="http://localhost:8080"
COOKIE_DIR="/tmp/wr_load_test"
RESULT_DIR="/tmp/wr_load_results"
CONCURRENCY=200  # 동시 curl 프로세스 수 제한

cleanup() {
    rm -rf "$COOKIE_DIR" "$RESULT_DIR"
}
trap cleanup EXIT

rm -rf "$COOKIE_DIR" "$RESULT_DIR"
mkdir -p "$COOKIE_DIR" "$RESULT_DIR"

# jq 존재 확인
if ! command -v jq &>/dev/null; then
    echo "Error: jq is required. Install with: brew install jq" >&2
    exit 1
fi

echo "=== Waiting Room Load Test ==="
echo "Total users: $TOTAL"
echo "Concurrency: $CONCURRENCY"
echo ""

# Phase 1: 1000명 동시 접속
echo "[Phase 1] Sending $TOTAL concurrent requests..."
START=$(date +%s%N)

send_request() {
    local i=$1
    curl -s -c "$COOKIE_DIR/user_$i.txt" -o "$RESULT_DIR/user_$i.html" \
        -w "%{http_code}" "$WR_URL/" > "$RESULT_DIR/user_${i}_status.txt" 2>/dev/null
}
export -f send_request
export COOKIE_DIR RESULT_DIR WR_URL

seq 1 "$TOTAL" | xargs -P "$CONCURRENCY" -I {} bash -c 'send_request "$@"' _ {} || true

END=$(date +%s%N)
ELAPSED=$(( (END - START) / 1000000 ))

# 결과 분석 (개별 cat/grep 대신 일괄 처리)
echo ""
echo "Analyzing results..."

ADMITTED=$(grep -rl "티켓 구매" "$RESULT_DIR"/user_*.html 2>/dev/null | wc -l | tr -d ' ')
QUEUED=$(grep -rl "Please wait" "$RESULT_DIR"/user_*.html 2>/dev/null | wc -l | tr -d ' ')
TOTAL_OK=$(grep -rl "^200$" "$RESULT_DIR"/user_*_status.txt 2>/dev/null | wc -l | tr -d ' ')
ERRORS=$((TOTAL - TOTAL_OK))

echo ""
echo "[Phase 1 Results] ${ELAPSED}ms elapsed"
echo "  Admitted (origin):  $ADMITTED"
echo "  Queued (waiting):   $QUEUED"
echo "  Errors:             $ERRORS"
echo ""

# 현재 상태
echo "[Queue Status]"
curl -s "$WR_URL/__wr/status" | jq .
echo ""

echo "=== Test Complete ==="
