#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/../../target/release/rhp"
f="$DIR/$1"
echo "===== PHP ====="
php "$f" 2>&1
echo "===== RHP ====="
"$BIN" "$f" 2>&1
echo "===== DIFF (php|rhp) ====="
diff <(php "$f" 2>&1) <("$BIN" "$f" 2>&1)
