#!/bin/bash
# 1000명 동시 접속 부하 테스트
# max_active=10, session_ttl=1s, reaper_interval=1s
# → 10명씩 ~1초 간격으로 순차 입장

TOTAL=1000
WR_URL="http://localhost:8080"
COOKIE_DIR="/tmp/wr_load_test"
RESULT_DIR="/tmp/wr_load_results"

rm -rf "$COOKIE_DIR" "$RESULT_DIR"
mkdir -p "$COOKIE_DIR" "$RESULT_DIR"

echo "=== Waiting Room Load Test ==="
echo "Total users: $TOTAL"
echo ""

# Phase 1: 1000명 동시 접속
echo "[Phase 1] Sending $TOTAL concurrent requests..."
START=$(date +%s%N)

for i in $(seq 1 $TOTAL); do
    curl -s -c "$COOKIE_DIR/user_$i.txt" -o "$RESULT_DIR/user_$i.html" \
        -w "%{http_code}" "$WR_URL/" > "$RESULT_DIR/user_${i}_status.txt" 2>/dev/null &
done

echo "Waiting for all requests to complete..."
wait

END=$(date +%s%N)
ELAPSED=$(( (END - START) / 1000000 ))

# 결과 분석
ADMITTED=0
QUEUED=0
ERRORS=0

for i in $(seq 1 $TOTAL); do
    status=$(cat "$RESULT_DIR/user_${i}_status.txt" 2>/dev/null)
    if [ "$status" = "200" ]; then
        if grep -q "티켓 구매" "$RESULT_DIR/user_$i.html" 2>/dev/null; then
            ADMITTED=$((ADMITTED + 1))
        elif grep -q "Please wait" "$RESULT_DIR/user_$i.html" 2>/dev/null; then
            QUEUED=$((QUEUED + 1))
        fi
    else
        ERRORS=$((ERRORS + 1))
    fi
done

echo ""
echo "[Phase 1 Results] ${ELAPSED}ms elapsed"
echo "  Admitted (origin):  $ADMITTED"
echo "  Queued (waiting):   $QUEUED"
echo "  Errors:             $ERRORS"
echo ""

# 현재 상태
echo "[Queue Status]"
curl -s "$WR_URL/__wr/status"
echo ""
echo ""

# Phase 2: 순차 입장 모니터링
echo "[Phase 2] Monitoring queue drain (10 users/~1s)..."
echo "  Queued users will be admitted as active sessions expire."
echo ""

for tick in $(seq 1 20); do
    sleep 1
    STATUS=$(curl -s "$WR_URL/__wr/status")
    ACTIVE=$(echo "$STATUS" | grep -o '"active_users":[0-9]*' | cut -d: -f2)
    QUEUE=$(echo "$STATUS" | grep -o '"queue_length":[0-9]*' | cut -d: -f2)
    TIMESTAMP=$(date +%H:%M:%S)
    printf "  [%s] tick=%2d  active=%-4s  queue=%-4s" "$TIMESTAMP" "$tick" "$ACTIVE" "$QUEUE"

    # 몇 명이 입장했는지 샘플 체크
    SAMPLE_ADMITTED=0
    for s in $(seq 1 20); do
        idx=$(( (tick - 1) * 20 + s ))
        if [ $idx -le $TOTAL ] && [ -f "$COOKIE_DIR/user_$idx.txt" ]; then
            RESP=$(curl -s -b "$COOKIE_DIR/user_$idx.txt" -o /dev/null -w "%{http_code}" "$WR_URL/")
            if [ "$RESP" = "200" ]; then
                BODY=$(curl -s -b "$COOKIE_DIR/user_$idx.txt" "$WR_URL/")
                if echo "$BODY" | grep -q "티켓 구매"; then
                    SAMPLE_ADMITTED=$((SAMPLE_ADMITTED + 1))
                fi
            fi
        fi
    done
    echo "  (sample: $SAMPLE_ADMITTED/20 admitted)"

    if [ "$QUEUE" = "0" ]; then
        echo ""
        echo "  Queue drained!"
        break
    fi
done

echo ""
echo "[Final Status]"
curl -s "$WR_URL/__wr/status"
echo ""

# 정리
rm -rf "$COOKIE_DIR" "$RESULT_DIR"
echo ""
echo "=== Test Complete ==="
