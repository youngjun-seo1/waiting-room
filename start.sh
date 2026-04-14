#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# macOS 튜닝 (부하 테스트 시 필요, sudo 필요)
# 아래 명령이 미적용 시 대규모 동시 연결에서 에러 발생:
#   sudo sysctl -w kern.ipc.somaxconn=8192              # TCP backlog (기본 128)
#   sudo sysctl -w net.inet.ip.portrange.first=10000    # 임시 포트 범위 확장 (기본 49152)
#   sudo launchctl limit maxfiles 65536 200000           # FD 제한 (기본 256)

# 파일 디스크립터 제한 상향 (SSE 동시 연결에 필요, macOS 기본값 256은 부족)
ulimit -n 65536
echo "ulimit -n: $(ulimit -n)"

# 사용법: ./start.sh [local|redis] [--debug]
MODE="local"
BUILD="release"

for arg in "$@"; do
  case "$arg" in
    redis) MODE="redis" ;;
    local) MODE="local" ;;
    --debug) BUILD="debug" ;;
  esac
done

REDIS_URL="${WR_REDIS_URL:-redis://127.0.0.1:6379}"

if [ "$BUILD" = "release" ]; then
  CARGO_RUN="run --release --bin waiting-room"
  CARGO_ORIGIN="run --release --example origin"
  BUILD_LABEL="release"
else
  CARGO_RUN="run --bin waiting-room"
  CARGO_ORIGIN="run --example origin"
  BUILD_LABEL="debug (hot reload)"
fi

PIDS=()

cleanup() {
  echo ""
  echo "Shutting down..."
  kill "${PIDS[@]}" 2>/dev/null
  wait "${PIDS[@]}" 2>/dev/null
  echo "All servers stopped."
}
trap cleanup EXIT

if [ "$BUILD" = "debug" ]; then
  # Debug: cargo watch로 hot reload
  echo "==> Starting origin server (port 3000, hot reload)..."
  cargo watch -w examples/ -x "run --example origin" &
  PIDS+=($!)
  sleep 3

  if [ "$MODE" = "redis" ]; then
    echo "==> Starting waiting-room server #1 (port 8080, Redis, hot reload)..."
    WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8080" cargo watch -w src/ -w config.toml -x "run --bin waiting-room -- config.toml" &
    PIDS+=($!)
    sleep 3

    echo "==> Starting waiting-room server #2 (port 8081, Redis, hot reload)..."
    WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8081" cargo watch -w src/ -w config.toml -x "run --bin waiting-room -- config.toml" &
    PIDS+=($!)
    sleep 3
  else
    echo "==> Starting waiting-room server (port 8080, hot reload)..."
    cargo watch -w src/ -w config.toml -x "run --bin waiting-room -- config.toml" &
    PIDS+=($!)
    sleep 3
  fi
else
  # Release: 빌드 후 직접 실행
  echo "==> Building release..."
  cargo build --release --bin waiting-room --example origin 2>&1

  echo "==> Starting origin server (port 3000)..."
  cargo $CARGO_ORIGIN &
  PIDS+=($!)
  sleep 1

  if [ "$MODE" = "redis" ]; then
    echo "==> Starting waiting-room server #1 (port 8080, Redis)..."
    WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8080" cargo $CARGO_RUN -- config.toml &
    PIDS+=($!)
    sleep 1

    echo "==> Starting waiting-room server #2 (port 8081, Redis)..."
    WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8081" cargo $CARGO_RUN -- config.toml &
    PIDS+=($!)
    sleep 1
  else
    echo "==> Starting waiting-room server (port 8080)..."
    cargo $CARGO_RUN -- config.toml &
    PIDS+=($!)
    sleep 1
  fi
fi

# Admin SPA
echo "==> Starting admin SPA (port 5173)..."
cd admin
npm run dev -- --host &
PIDS+=($!)
cd ..

echo ""
echo "============================================"
echo "  Build:        $BUILD_LABEL"
if [ "$MODE" = "redis" ]; then
  echo "  Mode:         Redis ($REDIS_URL)"
  echo "  Origin:       http://localhost:3000"
  echo "  Waiting Room: http://localhost:8080"
  echo "  Waiting Room: http://localhost:8081"
  echo "  Admin SPA:    http://localhost:5173"
else
  echo "  Mode:         In-Memory"
  echo "  Origin:       http://localhost:3000"
  echo "  Waiting Room: http://localhost:8080"
  echo "  Admin SPA:    http://localhost:5173"
fi
echo "============================================"
echo "Press Ctrl+C to stop all servers."
echo ""

wait
