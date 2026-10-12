#!/bin/bash
# Run the isolation-probe matrix: each attempt in its own keep, classified by
# the keep's return code. Writes RESULT lines to $OUT.
# Usage: run-matrix.sh <attest|noattest> <out-file>
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
mode=${1:-noattest}; OUT=${2:-/tmp/probe-matrix.out}
: > "$OUT"
# Under the stack the live cages are the grate(s) and the probe, with small
# ids; scan a few. self is reported for reference.
bash "$HERE/run1.sh" "$mode" self 2>&1 | grep RESULT >> "$OUT"
for t in 1 2 3 4; do
  for cmd in readcage writecage hijack; do
    bash "$HERE/run1.sh" "$mode" "$cmd" "$t" 0x10000 2>&1 | grep RESULT >> "$OUT"
  done
done
echo "MATRIX_DONE" >> "$OUT"
