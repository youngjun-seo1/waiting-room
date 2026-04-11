#!/bin/bash

check() {
  local name=$1 port=$2
  pid=$(lsof -ti:$port 2>/dev/null | head -1)
  if [ -n "$pid" ]; then
    echo "  $name (port $port) — running (pid $pid)"
  else
    echo "  $name (port $port) — stopped"
  fi
}

echo "Server Status"
echo "============="
check "Origin"         3000
check "Waiting Room"   8080
check "Waiting Room"   8081
check "Admin SPA"      5173
