#!/bin/bash
# Cold vs warm Lua builds on TriSeal and Gramine (encrypted and plain files),
# with native builds for reference.
#   cold     one build, from launching the TEE to its exit (lua-*.sh)
#   first    the first of two builds in one TEE instance (TWICE=1)
#   warm     the second build in that running instance: TriSeal's module
#            cache and IMFS are warm; Gramine still starts an enclave per
#            process
#   startup  launching the TEE and running `make --version` (PROBE=1)
# Usage: e2e/coldwarm.sh [runs]   -> $OUT/coldwarm.csv and one log per run
#   SYSTEMS  which of native, triseal, gramine to run (default all three)
#   GTAG     suffix for the Gramine rows, e.g. "-src" when a Gramine built
#            from source is first on PATH
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
N=${1:-3}
OUT=${OUT:-$HERE/../results/coldwarm-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
CSV=$OUT/coldwarm.csv
echo "system,run,kind,seconds,ok" >"$CSV"

# one <system> <run> <kind> <log> <cmd...>: run, then record wall-clock and
# the per-build times it printed.
one() {
  local sys=$1 run=$2 kind=$3 log=$OUT/$1.$3.$2.out
  shift 3
  echo "== $sys $kind run $run" >&2
  env "$@" >"$log" 2>&1
  local wall ok
  wall=$(sed -n 's/^wall \([0-9.]*\) rc=.*/\1/p' "$log")
  ok=$(grep -c '^Lua 5.5' "$log")
  case $kind in
    cold) echo "$sys,$run,cold,$wall,$ok" >>"$CSV" ;;
    startup) echo "$sys,$run,startup,$wall," >>"$CSV" ;;
    twice)
      echo "$sys,$run,twice-total,$wall,$ok" >>"$CSV"
      awk -v s="$sys" -v r="$run" '$1 == "@build" {
        print s "," r "," ($2 == 1 ? "first" : "warm") "," $4 "," ($5 == 0) }' "$log" >>"$CSV" ;;
  esac
}

SYSTEMS=${SYSTEMS:-native triseal gramine}
GTAG=${GTAG:-}
has() { [[ " $SYSTEMS " == *" $1 "* ]]; }
for i in $(seq "$N"); do
  if has native; then
    one native "$i" cold "$HERE/lua-native.sh" "$E/work-cw-native"
    one native "$i" twice TWICE=1 "$HERE/lua-native.sh" "$E/work-cw-native"
  fi
  if has triseal; then
    one triseal "$i" cold "$HERE/lua-triseal.sh" "$E/work-cw-triseal"
    one triseal "$i" twice TWICE=1 "$HERE/lua-triseal.sh" "$E/work-cw-triseal"
    one triseal "$i" startup PROBE=1 "$HERE/lua-triseal.sh" "$E/work-cw-triseal"
  fi
  if has gramine; then
    for mode in encrypted allowed; do
      g=gramine-$mode$GTAG
      one "$g" "$i" cold MODE=$mode "$HERE/lua-gramine.sh" "$E/work-cw-gramine"
      one "$g" "$i" twice MODE=$mode TWICE=1 "$HERE/lua-gramine.sh" "$E/work-cw-gramine"
      one "$g" "$i" startup MODE=$mode PROBE=1 "$HERE/lua-gramine.sh" "$E/work-cw-gramine"
    done
  fi
done

python3 - "$CSV" <<'PY'
import csv, statistics, sys
from collections import defaultdict
v = defaultdict(list)
for r in csv.DictReader(open(sys.argv[1])):
    if r["seconds"]:
        v[(r["system"], r["kind"])].append(float(r["seconds"]))
kinds = ["cold", "startup", "first", "warm", "twice-total"]
systems = list(dict.fromkeys(s for s, _ in v))
print("mean seconds (min-max)")
print(f"{'system':20s}" + "".join(f"{k:>22s}" for k in kinds))
for s in systems:
    cells = []
    for k in kinds:
        x = v.get((s, k), [])
        cells.append(f"{statistics.mean(x):.1f} ({min(x):.1f}-{max(x):.1f})" if x else "-")
    print(f"{s:20s}" + "".join(f"{c:>22s}" for c in cells))
PY
echo "results: $OUT"
