#!/bin/bash
# How much enclave memory the builds use at once, against how much the keep
# commits: the Lua, oggenc and gcc.c builds (preloads in shared memory) on
# two runtimes built from TriSeal branch triseal-working-set:
#   in-use       the shim counts committed pages that lie in a mapping
#                (commits ahead, as the release runtime does)
#   working-set  the same counts, with pages committed on first touch only,
#                one at a time: the pages the payload actually touches
# Each prints, at the IMFS phase mark and at exit, pages committed, pages in
# use now and their peak (lower bound: pages committed for the mapping they
# are in; upper bound: also committed pages a new mapping reuses).
# Usage: e2e/working-set.sh     (OUT=<dir>, WS=<dir with enarx-in-use and
#        enarx-working-set>, PROGS="lua oggenc gcc")
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
WS=${WS:-$E/ws}
OUT=${OUT:-$E/working-set-$(date +%Y%m%d-%H%M%S)}
PROGS=${PROGS:-lua oggenc gcc}
mkdir -p "$OUT"
export IMFS_GRATE=${IMFS_GRATE:-$HERE/../triseal/grates/ze/imfs-grate.cwasm}
export INTOTO_GRATE=${INTOTO_GRATE:-$HERE/../triseal/grates/ze/intoto-grate.cwasm}
for rt in in-use working-set; do
  for p in $PROGS; do
    echo "== $rt $p $(date +%T)" >&2
    W=$E/work-ws-$p
    if [ "$p" = lua ]; then
      ENARX_SGX_STATS=y ENARX=$WS/enarx-$rt "$HERE/lua-triseal-shm.sh" "$W" >"$OUT/$rt.$p.out" 2>&1
    else
      ENARX_SGX_STATS=y ENARX=$WS/enarx-$rt "$HERE/sfp/triseal-shm.sh" "$p" "$W" >"$OUT/$rt.$p.out" 2>&1
    fi
    cp "$W/build.log" "$OUT/$rt.$p.log"
    grep -E "^wall|ok|verified|FAIL" "$OUT/$rt.$p.out" | head -4 >&2
  done
done
python3 "$HERE/working-set.py" "$OUT" | tee "$OUT/summary.txt"
