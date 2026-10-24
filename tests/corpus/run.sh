#!/usr/bin/env bash
# Differential runner: compares php vs phrust VM output for every corpus/*.php
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/../../target/release/phrust"
pass=0; fail=0; failed=()
for f in "$DIR"/*.php; do
  php_out="$(php "$f" 2>&1)"
  vm_out="$("$BIN" "$f" 2>&1)"
  if [ "$php_out" == "$vm_out" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed+=("$(basename "$f")")
  fi
done
echo "PASS=$pass FAIL=$fail"
for x in "${failed[@]:-}"; do [ -n "$x" ] && echo "FAIL: $x"; done
