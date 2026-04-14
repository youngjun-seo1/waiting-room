#!/bin/bash
# SSE 동시 연결 수 테스트
#
# 사용법:
#   ./tests/run_conn_test.sh
#   ./tests/run_conn_test.sh --target 5000 --rate 500 --hold 60

set -e
ulimit -n 65536
cargo run --release --bin conn_test -- "$@"
