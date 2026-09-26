#!/bin/bash
# Wall-clock one workload on every system. Build each system first (README).
# Usage: ./run.sh <app> [runs]     e.g. ./run.sh forkexecv 5
# Output: "<system> <ms> rc=<exit code>", one line per run.
set -uo pipefail
APP=${1:-forkexecv}
N=${2:-5}
HERE=$(cd "$(dirname "$0")" && pwd)
LIND=${LIND:-$HOME/lind-wasm}
ENARX=${ENARX:-$HOME/enarx/target/release/enarx}
# Occlum has no fork(); use the posix_spawn variant when there is one.
OAPP=$APP
[ -f "$HERE/src/${APP}_spawn.c" ] && OAPP=${APP}_spawn

time_ms() {
  local s rc
  s=$(date +%s%N)
  "$@" >/dev/null 2>&1
  rc=$?
  echo "$(( ($(date +%s%N) - s) / 1000000 )) rc=$rc"
}

bench() {
  local name=$1
  shift
  for _ in $(seq "$N"); do
    echo "$name $(time_ms "$@")"
  done
}

bench native         bash -c "cd '$HERE/gramine/app' && ./$APP"
bench gramine-direct bash -c "cd '$HERE/gramine' && gramine-direct $APP"
bench gramine-sgx    bash -c "cd '$HERE/gramine' && gramine-sgx $APP"
bench triseal-sgx    env ENARX_BACKEND=sgx \
  ENARX_WASMCFGFILE="$HERE/triseal/build/Enarx.$APP.toml" \
  "$ENARX" run "$LIND/lindfs/grates/imfs-grate.cwasm"

# Timed inside the container so docker startup is not counted.
"$HERE/occlum/run-docker.sh" bash -c "cd inst && for _ in \$(seq $N); do
  s=\$(date +%s%N); occlum run /bin/$OAPP >/dev/null 2>&1; rc=\$?
  echo \"occlum-sgx \$(( (\$(date +%s%N) - s) / 1000000 )) rc=\$rc\"; done"
