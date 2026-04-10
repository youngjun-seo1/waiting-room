#!/bin/bash
# 점진적 동접 한계 테스트
# 1000 → 5000 → 10000 → 20000 → 50000

WR_URL="http://localhost:8080"

test_concurrent() {
    local N=$1
    local COOKIE_DIR="/tmp/wr_stress_$N"
    rm -rf "$COOKIE_DIR"
    mkdir -p "$COOKIE_DIR"

    # flush
    curl -s -X POST -H "X-Api-Key: change-me-in-production" "$WR_URL/__wr/admin/flush" > /dev/null

    echo -n "[$N users] "
    START=$(date +%s%N)

    for i in $(seq 1 $N); do
        curl -s -c "$COOKIE_DIR/$i.txt" -o /dev/null -w "" "$WR_URL/" &
    done
    wait

    END=$(date +%s%N)
    ELAPSED=$(( (END - START) / 1000000 ))

    STATUS=$(curl -s "$WR_URL/__wr/status")
    ACTIVE=$(echo "$STATUS" | grep -o '"active_users":[0-9]*' | cut -d: -f2)
    QUEUE=$(echo "$STATUS" | grep -o '"queue_length":[0-9]*' | cut -d: -f2)
    TOTAL=$((ACTIVE + QUEUE))

    RPS=$(( N * 1000 / ELAPSED ))

    echo "${ELAPSED}ms  active=$ACTIVE  queue=$QUEUE  total=$TOTAL  ~${RPS} req/s"

    # 서버 메모리
    WR_PID=$(lsof -ti :8080 2>/dev/null | head -1)
    if [ -n "$WR_PID" ]; then
        RSS=$(ps -o rss= -p "$WR_PID" | tr -d ' ')
        RSS_MB=$((RSS / 1024))
        echo "         memory: ${RSS_MB}MB RSS"
    fi

    rm -rf "$COOKIE_DIR"
    sleep 2
}

echo "=== Stress Test ==="
echo "System: $(sysctl -n hw.ncpu) cores, $(($(sysctl -n hw.memsize) / 1024 / 1024 / 1024))GB RAM"
echo "fd limit: $(ulimit -n)"
echo ""

for N in 1000 5000 10000 20000 50000; do
    test_concurrent $N
    # 서버가 살아있는지 확인
    if ! curl -s "$WR_URL/__wr/status" > /dev/null 2>&1; then
        echo "SERVER DOWN at $N users!"
        break
    fi
done

echo ""
echo "=== Done ==="
