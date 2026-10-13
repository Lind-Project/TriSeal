#!/bin/bash
# Every handler-table probe on r3 (no check) and auth (checked), bare and under IMFS.
# Usage: run-auth-matrix.sh <out-file>
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${1:-$HOME/sgx-bench/e2e/work-auth/matrix.out}
mkdir -p "$(dirname "$OUT")"; : > "$OUT"
for rt in r3 auth; do
  for cmd in hijack-parent native-ptr spoof-open; do
    bash "$HERE/run-auth.sh" $rt bare $cmd >> "$OUT" 2>&1
  done
  for cmd in hijack-parent hijack-grate copy-escape native-ptr spoof-open; do
    bash "$HERE/run-auth.sh" $rt imfs $cmd >> "$OUT" 2>&1
  done
done
echo MATRIX_DONE >> "$OUT"
