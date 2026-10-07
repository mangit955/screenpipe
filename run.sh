#!/usr/bin/env bash
# Run each scenario in a fresh process with a hard 20 s timeout.
set -u
BIN=./target/debug/wv-harness
SCENARIOS=${SCENARIOS:-"data-literal data surrogate nav"}
for scenario in $SCENARIOS; do
  for mode in control fix; do
    echo "===== BEGIN scenario=$scenario mode=$mode ====="
    "$BIN" "$scenario" "$mode" 2>&1 &
    pid=$!
    for i in $(seq 1 200); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
      echo "HUNG after 20 s; backtrace:"
      sudo gdb -p "$pid" -batch -ex "thread apply all bt 25" 2>&1 | grep -E '^(Thread|#)' | head -120
      kill -KILL "$pid"
    fi
    wait "$pid"
    code=$?
    echo "===== END scenario=$scenario mode=$mode exit_code=$code ====="
  done
done
exit 0
