#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/../../target/release/phrust"
OUT="${1:-/tmp/phrust_diffs.txt}"
: > "$OUT"
for f in "$DIR"/*.php; do
  po="$(php "$f" 2>&1)"
  vo="$("$BIN" "$f" 2>&1)"
  if [ "$po" != "$vo" ]; then
    {
      echo "######## $(basename "$f") ########"
      diff <(printf '%s' "$po") <(printf '%s' "$vo")
      echo
    } >> "$OUT"
  fi
done
echo "wrote $OUT"
