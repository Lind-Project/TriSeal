#!/bin/bash
# Enclave transitions during the Lua build on TriSeal and Gramine, counted
# from the start of the compile: each system runs the build once and once
# with PROBE=1 (TEE startup plus preloading, then `make --version`); the
# difference is the compile.
#   TriSeal  ENARX_SGX_STATS=1 with the stats-enabled runtime (ENARX_STATS)
#   Gramine  sgx.enable_stats with a debugoptimized Gramine 1.8 (GRAMINE_DBG)
#   both     host syscalls with `strace -f -c`, using the timing builds
# Usage: e2e/counts.sh   -> $OUT/counts.csv and the raw logs
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
OUT=${OUT:-$HERE/../results/counts-$(date +%Y%m%d-%H%M%S)}
ENARX_STATS=${ENARX_STATS:-$E/ts/TriSeal36s/target/release/enarx}
GRAMINE_DBG=${GRAMINE_DBG:-$HOME/sgx-bench/gramine-dbg}
mkdir -p "$OUT"

for kind in build probe; do
  probe=0
  [ $kind = probe ] && probe=1
  echo "== triseal stats $kind" >&2
  ENARX_SGX_STATS=1 PROBE=$probe ENARX=$ENARX_STATS "$HERE/lua-triseal.sh" "$E/work-cnt-triseal" >"$OUT/triseal-stats.$kind.out" 2>&1
  cp "$E/work-cnt-triseal/build.log" "$OUT/triseal-stats.$kind.log"

  echo "== gramine stats $kind" >&2
  PATH=$GRAMINE_DBG/bin:$PATH \
    PYTHONPATH=$(echo "$GRAMINE_DBG"/lib/python3*/site-packages) \
    STATS=1 PROBE=$probe MODE=encrypted "$HERE/lua-gramine.sh" "$E/work-cnt-gramine" >"$OUT/gramine-stats.$kind.out" 2>&1
  cp "$E/work-cnt-gramine.log" "$OUT/gramine-stats.$kind.log"

  echo "== triseal strace $kind" >&2
  TRACE="strace -f -c -o $OUT/triseal-strace.$kind.txt" PROBE=$probe "$HERE/lua-triseal.sh" "$E/work-cnt-triseal" >"$OUT/triseal-strace.$kind.out" 2>&1

  echo "== gramine strace $kind" >&2
  TRACE="strace -f -c -o $OUT/gramine-strace.$kind.txt" PROBE=$probe MODE=encrypted "$HERE/lua-gramine.sh" "$E/work-cnt-gramine" >"$OUT/gramine-strace.$kind.out" 2>&1
done

python3 "$HERE/counts-summary.py" "$OUT"
echo "results: $OUT"
