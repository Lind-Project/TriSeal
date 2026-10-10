#!/bin/bash
# Time the Lua build in every configuration and collect the results.
# Usage: e2e/run-all.sh [runs]   -> results/e2e-<timestamp>/{times.csv,*.out}
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
N=${1:-3}
OUT=$HERE/../results/e2e-$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
echo "config,run,wall_s,rc,ok" >"$OUT/times.csv"

# one <config> <run> <command...>: run it, keep its output, record wall and
# whether the resulting lua ran.
one() {
  local cfg=$1 run=$2 log
  shift 2
  log=$OUT/$cfg.$run.out
  echo "== $cfg run $run" >&2
  env "$@" >"$log" 2>&1
  local wall rc ok
  wall=$(sed -n 's/^wall \([0-9.]*\) rc=.*/\1/p' "$log")
  rc=$(sed -n 's/^wall [0-9.]* rc=\(.*\)/\1/p' "$log")
  ok=$(grep -c '^Lua 5.5' "$log")
  echo "$cfg,$run,$wall,$rc,$ok" >>"$OUT/times.csv"
}

for i in $(seq "$N"); do
  one triseal "$i" "$HERE/lua-triseal.sh"
done
one triseal-intoto 1 INTOTO=1 "$HERE/lua-triseal.sh"
one triseal-probe 1 PROBE=1 "$HERE/lua-triseal.sh"
for i in $(seq "$N"); do
  one gramine-encrypted "$i" MODE=encrypted "$HERE/lua-gramine.sh"
done
for i in $(seq "$N"); do
  one gramine-allowed "$i" MODE=allowed "$HERE/lua-gramine.sh"
done
for i in $(seq "$N"); do
  one gramine-encrypted-edmm "$i" MODE=encrypted EDMM=true "$HERE/lua-gramine.sh"
done
cat "$OUT/times.csv"
echo "results: $OUT"
