#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/../../target/release/phrust"
f="$DIR/$1"
echo "===== PHP ====="
php "$f" 2>&1
echo "===== PHRUST ====="
"$BIN" "$f" 2>&1
echo "===== DIFF (php|phrust) ====="
diff <(php "$f" 2>&1) <("$BIN" "$f" 2>&1)
