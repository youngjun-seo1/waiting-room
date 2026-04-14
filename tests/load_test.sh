#!/bin/bash
# SSE 기반 대기열 부하 테스트
# 각 유저: GET / → 쿠키 획득 → 즉시 SSE → admit 대기
# 전원을 백그라운드로 동시 실행하여 xargs 병목 제거
#
# 사용법: ./load_test.sh [TOTAL]

set -uo pipefail

TOTAL="${1:-1000}"
WR_URL="http://localhost:8080"
COUNTER_DIR="/tmp/wr_load_counters"
SSE_TIMEOUT=120

cleanup() {
    rm -rf "$COUNTER_DIR"
    # 남은 백그라운드 curl 정리
    jobs -p 2>/dev/null | xargs kill 2>/dev/null || true
}
trap cleanup EXIT

rm -rf "$COUNTER_DIR"
mkdir -p "$COUNTER_DIR"

for f in admitted_direct admitted_sse timeout error; do
    > "$COUNTER_DIR/$f"
done

if ! command -v jq &>/dev/null; then
    echo "Error: jq is required. Install with: brew install jq" >&2
    exit 1
fi

echo "=== Waiting Room SSE Load Test ==="
echo "Total users: $TOTAL"
echo "SSE timeout: ${SSE_TIMEOUT}s"
echo ""

# Waiting room 활성화 확인
WR_STATUS=$(curl -s "$WR_URL/__wr/status" | jq -r '.enabled')
if [[ "$WR_STATUS" != "true" ]]; then
    echo "Error: Waiting room is disabled. Create a schedule first." >&2
    exit 1
fi
echo "Waiting room is enabled."
echo ""

simulate_user() {
    local i=$1
    local max_retries=3
    local attempt=0

    while [[ "$attempt" -lt "$max_retries" ]]; do
        attempt=$((attempt + 1))

        # Step 1: GET / → 쿠키 획득
        local tmpjar
        tmpjar=$(mktemp)
        local http_code
        http_code=$(curl -s -c "$tmpjar" \
            -w "%{http_code}" -o /dev/null "$WR_URL/" 2>/dev/null)

        case "$http_code" in
            302)
                echo 1 >> "$COUNTER_DIR/admitted_direct"
                rm -f "$tmpjar"
                return
                ;;
            200)
                ;;
            *)
                echo 1 >> "$COUNTER_DIR/error"
                rm -f "$tmpjar"
                return
                ;;
        esac

        local token
        token=$(grep "__wr_token" "$tmpjar" 2>/dev/null | awk '{print $NF}')
        rm -f "$tmpjar"

        if [[ -z "$token" ]]; then
            echo 1 >> "$COUNTER_DIR/error"
            return
        fi

        # Step 2: 즉시 SSE 연결 → admit 대기
        local admitted=0
        local got_any=0
        while IFS= read -r line; do
            if [[ -n "$line" ]]; then
                got_any=1
            fi
            if [[ "$line" == *'"admit"'* ]]; then
                admitted=1
                break
            fi
        done < <(timeout "$SSE_TIMEOUT" curl -s -N \
            -H "Cookie: __wr_token=$token" \
            "$WR_URL/__wr/events" 2>/dev/null)

        if [[ "$admitted" -eq 1 ]]; then
            echo "[SSE] user $i admitted"
            echo 1 >> "$COUNTER_DIR/admitted_sse"
            return
        fi

        # SSE에서 데이터를 못 받음 → 세션 만료, 재시도
        if [[ "$got_any" -eq 0 && "$attempt" -lt "$max_retries" ]]; then
            echo "[SSE] user $i: no data, retrying ($attempt/$max_retries)"
            continue
        fi

        # 데이터는 받았지만 admit 안 옴 → 진짜 timeout
        break
    done

    echo "[SSE] user $i timeout"
    echo 1 >> "$COUNTER_DIR/timeout"
}

echo "Launching $TOTAL users..."
START=$(date +%s%N)

PIDS=()
for i in $(seq 1 "$TOTAL"); do
    simulate_user "$i" &
    PIDS+=($!)

    # ulimit 보호: 동시 프로세스가 너무 많으면 잠시 대기
    # 각 유저가 2개 프로세스(bash + curl) 사용하므로 절반으로 제한
    if (( ${#PIDS[@]} % 500 == 0 )); then
        # 끝난 프로세스 정리
        local_pids=()
        for pid in "${PIDS[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then
                local_pids+=("$pid")
            fi
        done
        PIDS=("${local_pids[@]}")
    fi
done

echo "All $TOTAL users launched. Waiting for completion..."
echo ""

# 모든 백그라운드 프로세스 대기
wait

END=$(date +%s%N)
ELAPSED=$(( (END - START) / 1000000 ))

# 결과 집계
DIRECT=$(wc -l < "$COUNTER_DIR/admitted_direct" | tr -d ' ')
SSE=$(wc -l < "$COUNTER_DIR/admitted_sse" | tr -d ' ')
TIMEOUTS=$(wc -l < "$COUNTER_DIR/timeout" | tr -d ' ')
ERRORS=$(wc -l < "$COUNTER_DIR/error" | tr -d ' ')

echo ""
echo "[Results] ${ELAPSED}ms elapsed"
echo "  Admitted (direct):  $DIRECT"
echo "  Admitted (SSE):     $SSE"
echo "  Timeout:            $TIMEOUTS"
echo "  Errors:             $ERRORS"
echo "  Total:              $((DIRECT + SSE + TIMEOUTS + ERRORS)) / $TOTAL"

echo ""
echo "[Queue Status]"
curl -s "$WR_URL/__wr/status" | jq .
echo ""

echo "=== Test Complete ==="
