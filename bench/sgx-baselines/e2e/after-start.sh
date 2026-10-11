#!/bin/bash
# Enclave transitions from the start of the Lua build to its end, on TriSeal
# and Gramine (the build's cost, not the TEE's startup or the preloading).
#
#   TriSeal  the stats runtime (ENARX_STATS, branch triseal-zero-ecall-stats)
#            prints its counters when the IMFS grate marks phase 1, as it
#            starts make after staging the preloads, and again at exit; the
#            difference is what the build asked of the host. Two setups:
#              triseal-preloads  lua-triseal.sh: IMFS stages the host files
#                                listed in PRELOADS, then runs make
#              triseal-shm       lua-triseal-shm.sh: preloads from shared
#                                memory, two in-toto attestations, the dump
#                                committed by the host at exit
#   Gramine  (encrypted files) has no such mark: build minus probe (`make
#            --version`), from sgx.enable_stats of a debugoptimized Gramine
#            (GRAMINE_DBG), every enclave (one per process) counted.
#
# Usage: OUT=<dir> e2e/after-start.sh     (then after-start.py <dir>)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
OUT=${OUT:-$E/after-start}
ENARX_STATS=${ENARX_STATS:-$E/ze/TriSeal-s/target/release/enarx}
GRAMINE_DBG=${GRAMINE_DBG:-$HOME/sgx-bench/gramine-dbg}
mkdir -p "$OUT"

echo "== triseal-preloads" >&2
ENARX_SGX_STATS=y ENARX=$ENARX_STATS "$HERE/lua-triseal.sh" "$E/work-as-triseal" \
  >"$OUT/triseal-preloads.out" 2>&1
cp "$E/work-as-triseal/build.log" "$OUT/triseal-preloads.log"

echo "== triseal-shm" >&2
ENARX_SGX_STATS=y ENARX=$ENARX_STATS "$HERE/lua-triseal-shm.sh" "$E/work-as-triseal-shm" \
  >"$OUT/triseal-shm.out" 2>&1
cp "$E/work-as-triseal-shm/build.log" "$OUT/triseal-shm.log"

for kind in build probe; do
  probe=0
  [ $kind = probe ] && probe=1
  echo "== gramine $kind" >&2
  PATH=$GRAMINE_DBG/bin:$PATH \
    PYTHONPATH=$(echo "$GRAMINE_DBG"/lib/python3*/site-packages) \
    STATS=1 PROBE=$probe MODE=encrypted "$HERE/lua-gramine.sh" "$E/work-as-gramine" \
    >"$OUT/gramine.$kind.out" 2>&1
  cp "$E/work-as-gramine.log" "$OUT/gramine.$kind.log"
done

python3 "$HERE/after-start.py" "$OUT"
