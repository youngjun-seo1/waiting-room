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

# 1) Origin server (port 3000) - hot reload on examples/ changes
echo "==> Starting origin server (port 3000, hot reload)..."
cargo watch -w examples/ -x "run --example origin" &
PIDS+=($!)
sleep 3

if [ "$MODE" = "redis" ]; then
  # Redis mode: 2 WR servers - hot reload on src/ changes
  echo "==> Starting waiting-room server #1 (port 8080, Redis, hot reload)..."
  WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8080" cargo watch -w src/ -w config.toml -x "run -- config.toml" &
  PIDS+=($!)
  sleep 3

  echo "==> Starting waiting-room server #2 (port 8081, Redis, hot reload)..."
  WR_REDIS_URL="$REDIS_URL" WR_LISTEN_ADDR="0.0.0.0:8081" cargo watch -w src/ -w config.toml -x "run -- config.toml" &
  PIDS+=($!)
  sleep 3
else
  # Local mode: 1 WR server (in-memory) - hot reload on src/ changes
  echo "==> Starting waiting-room server (port 8080, hot reload)..."
  cargo watch -w src/ -w config.toml -x "run -- config.toml" &
  PIDS+=($!)
  sleep 3
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
