#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

MODE="${1:-local}"
REDIS_URL="${WR_REDIS_URL:-redis://127.0.0.1:6379}"

PIDS=()

cleanup() {
  echo ""
  echo "Shutting down..."
  kill "${PIDS[@]}" 2>/dev/null
  wait "${PIDS[@]}" 2>/dev/null
  echo "All servers stopped."
}
trap cleanup EXIT

# Build
echo "==> Building waiting-room..."
cargo build --release 2>&1

# 1) Origin server (port 3000)
echo "==> Starting origin server (port 3000)..."
cargo run --release --example origin &
PIDS+=($!)
sleep 1

if [ "$MODE" = "redis" ]; then
  # Redis mode: 2 WR servers
  echo "==> Starting waiting-room server #1 (port 8080, Redis)..."
  WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8080" ./target/release/waiting-room config.toml &
  PIDS+=($!)
  sleep 1

  echo "==> Starting waiting-room server #2 (port 8081, Redis)..."
  WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8081" ./target/release/waiting-room config.toml &
  PIDS+=($!)
  sleep 1
else
  # Local mode: 1 WR server (in-memory)
  echo "==> Starting waiting-room server (port 8080)..."
  ./target/release/waiting-room config.toml &
  PIDS+=($!)
  sleep 1
fi

# Admin SPA
echo "==> Starting admin SPA (port 5173)..."
cd admin
npm run dev -- --host &
PIDS+=($!)
cd ..

echo ""
echo "============================================"
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
