#!/bin/bash
# 부하 테스트 리소스 모니터링
# waiting-room(멀티 서버 지원), redis-server, load_test/conn_test의 CPU/RSS/FD를 1초마다 출력
#
# 사용법:
#   ./tests/monitor.sh           # 터미널 출력
#   ./tests/monitor.sh --csv     # CSV 파일로 저장

set -u

CSV_MODE=false
CSV_FILE=""
if [[ "${1:-}" == "--csv" ]]; then
    CSV_MODE=true
    CSV_FILE="monitor_$(date +%Y%m%d_%H%M%S).csv"
fi

# 컬럼 너비 정의
COL_TIME=10
COL_CPU=7
COL_RSS=7
COL_FDS=7
COL_SEP=" │ "

get_proc_stats() {
    local pid=$1
    local stats
    stats=$(ps -p "$pid" -o %cpu=,rss= 2>/dev/null)
    if [[ -z "$stats" ]]; then
        echo "-:-:-"
        return
    fi
    local cpu rss_kb rss_mb fds
    cpu=$(echo "$stats" | awk '{print $1}')
    rss_kb=$(echo "$stats" | awk '{print $2}')
    rss_mb=$((rss_kb / 1024))
    fds=$(lsof -p "$pid" 2>/dev/null | wc -l | tr -d ' ')
    echo "${cpu}:${rss_mb}:${fds}"
}

get_wr_pids() {
    pgrep -f "target/release/waiting-room" 2>/dev/null
}

get_wr_port() {
    local pid=$1
    lsof -P -i TCP -sTCP:LISTEN 2>/dev/null | awk -v p="$pid" '$2==p{print $9}' | head -1 | sed 's/.*://'
}

get_redis_connections() {
    command redis-cli info clients 2>/dev/null | grep connected_clients | cut -d: -f2 | tr -d '[:space:]'
}

get_test_pid() {
    local pid
    pid=$(pgrep -f "target/release/load_test" 2>/dev/null | head -1)
    if [[ -z "$pid" ]]; then
        pid=$(pgrep -f "target/release/conn_test" 2>/dev/null | head -1)
    fi
    echo "$pid"
}

fmt_cell() {
    # 3개 값(CPU, RSS, FDs)을 고정 너비로 포맷
    local cpu=$1 rss=$2 extra=$3
    printf "%6s%% %4sMB %6s" "$cpu" "$rss" "$extra"
}

fmt_cell_short() {
    # 2개 값(CPU, RSS)을 고정 너비로 포맷
    local cpu=$1 rss=$2
    printf "%6s%% %4sMB" "$cpu" "$rss"
}

# waiting-room 서버 감지
WR_PIDS=($(get_wr_pids))
WR_COUNT=${#WR_PIDS[@]}

# 각 서버 포트 저장
WR_PORTS=()
for pid in "${WR_PIDS[@]}"; do
    WR_PORTS+=($(get_wr_port "$pid"))
done

print_header() {
    if $CSV_MODE; then
        local csv_header="time"
        for i in $(seq 1 $WR_COUNT); do
            csv_header+=",wr${i}_cpu%,wr${i}_rss_mb,wr${i}_fds"
        done
        csv_header+=",redis_cpu%,redis_rss_mb,redis_clients,test_cpu%,test_rss_mb"
        echo "$csv_header" > "$CSV_FILE"
        echo "Logging to $CSV_FILE"
    fi

    # 헤더 라인 1: 프로세스 이름
    printf "\033[1m"
    printf "%-10s" "TIME"
    for i in $(seq 0 $((WR_COUNT - 1))); do
        printf " │ %-21s" "WR:${WR_PORTS[$i]:-?}"
    done
    printf " │ %-21s" "redis-server"
    printf " │ %-14s" "load_test"
    echo ""

    # 헤더 라인 2: 컬럼 이름
    printf "%-10s" ""
    for i in $(seq 0 $((WR_COUNT - 1))); do
        printf " │ %6s  %5s  %6s" "CPU" "RSS" "FDs"
    done
    printf " │ %6s  %5s  %6s" "CPU" "RSS" "Cli"
    printf " │ %6s  %5s" "CPU" "RSS"
    printf "\033[0m"
    echo ""

    # 구분선
    local width=$((10 + (WR_COUNT + 1) * 24 + 17))
    printf "%.0s─" $(seq 1 $width)
    echo ""
}

print_row() {
    local time=$1

    # PID 갱신
    WR_PIDS=($(get_wr_pids))
    WR_COUNT=${#WR_PIDS[@]}

    printf "%-10s" "$time"
    local csv_row="$time"

    # waiting-room 서버들
    for i in $(seq 0 $((WR_COUNT - 1))); do
        local stats
        stats=$(get_proc_stats "${WR_PIDS[$i]}")
        local cpu rss fds
        IFS=: read -r cpu rss fds <<< "$stats"
        echo -n " │ $(fmt_cell "$cpu" "$rss" "$fds")"
        csv_row+=",$cpu,$rss,$fds"
    done
    if [[ $WR_COUNT -eq 0 ]]; then
        printf " │ %21s" "(not running)"
        csv_row+=",,,,"
    fi

    # redis
    local redis_pid
    redis_pid=$(pgrep -f "redis-server" 2>/dev/null | head -1)
    if [[ -n "$redis_pid" ]]; then
        local stats
        stats=$(get_proc_stats "$redis_pid")
        local cpu rss fds
        IFS=: read -r cpu rss fds <<< "$stats"
        local clients
        clients=$(get_redis_connections)
        echo -n " │ $(fmt_cell "$cpu" "$rss" "${clients:-0}")"
        csv_row+=",$cpu,$rss,${clients:-0}"
    else
        printf " │ %21s" "(not running)"
        csv_row+=",,,,"
    fi

    # test
    local test_pid
    test_pid=$(get_test_pid)
    if [[ -n "$test_pid" ]]; then
        local stats
        stats=$(get_proc_stats "$test_pid")
        local cpu rss fds
        IFS=: read -r cpu rss fds <<< "$stats"
        echo -n " │ $(fmt_cell_short "$cpu" "$rss")"
        csv_row+=",$cpu,$rss"
    else
        printf " │ %14s" "(stopped)"
        csv_row+=",,"
    fi

    echo ""

    if $CSV_MODE; then
        echo "$csv_row" >> "$CSV_FILE"
    fi
}

echo "Monitoring... (Ctrl+C to stop)"
if [[ $WR_COUNT -gt 1 ]]; then
    echo "Detected $WR_COUNT waiting-room servers"
fi
echo ""
print_header

while true; do
    print_row "$(date +%H:%M:%S)"
    sleep 1
done
