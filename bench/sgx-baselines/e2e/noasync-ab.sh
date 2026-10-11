#!/bin/bash
# cc1, as and ld without asyncify, against the toolchain as built today.
# Builds (preloads in shared memory, attested) on three setups, interleaved:
#   A  release runtime, toolchain $E/root (asyncify everywhere)
#   B  runtime with lind exiting without asyncify, the same toolchain
#   C  that runtime, $E/root-na: cc1, as and ld without asyncify
# Usage: e2e/noasync-ab.sh     (OUT=<dir>, N=<runs of lua and oggenc>,
#        ENARX_A=, ENARX_NA=, ROOT_NA=)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
OUT=${OUT:-$E/noasync-ab-$(date +%Y%m%d-%H%M%S)}
N=${N:-2}
ENARX_A=${ENARX_A:-$E/ze/TriSeal/target/release/enarx}
ENARX_NA=${ENARX_NA:-$E/na/TriSeal/target/release/enarx}
ROOT_NA=${ROOT_NA:-$E/root-na}
mkdir -p "$OUT"
CSV=$OUT/runs.csv
[ -f "$CSV" ] || echo "setup,prog,run,seconds,ok" >"$CSV"

one() {
  local setup=$1 p=$2 i=$3 enarx root f
  case $setup in
    A) enarx=$ENARX_A root=$E/root ;;
    B) enarx=$ENARX_NA root=$E/root ;;
    C) enarx=$ENARX_NA root=$ROOT_NA ;;
  esac
  f=$OUT/$setup.$p.$i.out
  echo "== $setup $p $i $(date +%T)" >&2
  if [ "$p" = lua ]; then
    ENARX=$enarx ROOT=$root "$HERE/lua-triseal-shm.sh" "$E/work-ab-$setup" >"$f" 2>&1
  else
    ENARX=$enarx ROOT=$root "$HERE/sfp/triseal-shm.sh" "$p" "$E/work-ab-$setup" >"$f" 2>&1
  fi
  local wall ok
  wall=$(sed -n 's/^wall \([0-9.]*\).*/\1/p' "$f" | tail -1)
  ok=$(grep -cE '"ok"|: ok|attestations verified' "$f")
  echo "$setup,$p,$i,$wall,$ok" >>"$CSV"
}

for i in $(seq "$N"); do
  for p in lua oggenc; do
    for s in A B C; do
      [ "$s" = B ] && [ "$i" -gt 1 ] && continue
      one $s $p "$i"
    done
  done
done
for s in A C; do one $s gcc 1; done
echo "results: $CSV"
