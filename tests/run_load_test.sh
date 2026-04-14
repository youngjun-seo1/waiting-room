#!/bin/bash
# SSE 시나리오 부하 테스트 (GET / → SSE → admit)
#
# 사용법:
#   ./tests/run_load_test.sh
#   ./tests/run_load_test.sh --total 10000 --concurrency 2000

set -e
ulimit -n 65536
cargo run --release --bin load_test -- "$@"
