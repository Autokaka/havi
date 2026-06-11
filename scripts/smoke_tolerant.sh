#!/usr/bin/env bash
# Regression guard for the tolerant render path (warmup-reload sequencing).
# Renders each example in --tolerant mode and fails if any stalls past warmup.
#   ./scripts/smoke_tolerant.sh [havi-binary]
set -uo pipefail
cd "$(dirname "$0")/.."

HAVI="${1:-dist/darwin-arm64/havi.app/Contents/MacOS/havi}"
[ -x "$HAVI" ] || { echo "havi binary not found: $HAVI"; exit 1; }

RUNS=3
fail=0
for html in examples/*.html; do
  [ "$(basename "$html")" = "stamp.html" ] && continue  # determinism fixture, not a page
  for r in $(seq 1 "$RUNS"); do
    out="$(mktemp -d)/out.mp4"
    log=$(HAVI_IPC=1 "$HAVI" "$(pwd)/$html" -t 12 -W 640 -H 360 -f 30 --tolerant -o "$out" 2>&1)
    if grep -qE 'timeout|stalled' <<<"$log"; then
      echo "STALL  $(basename "$html") run $r"
      fail=1
    fi
  done
  echo "ok     $(basename "$html")"
done

[ "$fail" -eq 0 ] && echo "all tolerant renders clean" || { echo "tolerant regression detected"; exit 1; }
