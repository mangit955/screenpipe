#!/usr/bin/env bash
# Run each scenario in a fresh process with a hard 20 s timeout.
set -u
BIN=./target/debug/wv-harness
for scenario in data-literal data surrogate nav; do
  for mode in control fix; do
    echo "===== BEGIN scenario=$scenario mode=$mode ====="
    timeout -s KILL 20 "$BIN" "$scenario" "$mode" 2>&1
    code=$?
    echo "===== END scenario=$scenario mode=$mode exit_code=$code ====="
  done
done
exit 0
