#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

cleanup() {
  echo ""
  echo "Shutting down..."
  kill $PID_ORIGIN $PID_WR $PID_ADMIN 2>/dev/null
  wait $PID_ORIGIN $PID_WR $PID_ADMIN 2>/dev/null
  echo "All servers stopped."
}
trap cleanup EXIT

# Build
echo "==> Building waiting-room..."
cargo build --release 2>&1

# 1) Origin server (port 3000)
echo "==> Starting origin server (port 3000)..."
cargo run --release --example origin &
PID_ORIGIN=$!
sleep 1

# 2) Waiting Room server (port 8080)
echo "==> Starting waiting-room server (port 8080)..."
./target/release/waiting-room config.toml &
PID_WR=$!
sleep 1

# 3) Admin SPA (port 5173)
echo "==> Starting admin SPA (port 5173)..."
cd admin
npm run dev -- --host &
PID_ADMIN=$!
cd ..

echo ""
echo "============================================"
echo "  Origin:      http://localhost:3000"
echo "  Waiting Room: http://localhost:8080"
echo "  Admin SPA:    http://localhost:5173"
echo "============================================"
echo "Press Ctrl+C to stop all servers."
echo ""

wait
