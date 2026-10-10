#!/bin/bash
# Run the in-process microbenchmarks (src/microbench.c) on every system and
# save the results. Build first:
#   (cd gramine && make APP=microbench)
#   triseal/build.sh microbench
#   occlum/run-docker.sh ./build.sh microbench
# Usage: ./run-micro.sh [syscall_iters] [proc_iters] [system...]
#   systems: native gramine-direct gramine-sgx triseal-sgx occlum-sgx (default: all)
# Writes results/<timestamp>/: micro.csv, one <system>.log each, env.txt.
set -uo pipefail
SYS_N=${1:-100000}
PROC_N=${2:-20}
shift 2 2>/dev/null || shift $#
SYSTEMS=${*:-native gramine-direct gramine-sgx triseal-sgx occlum-sgx}
HERE=$(cd "$(dirname "$0")" && pwd)
LIND=${LIND:-$HOME/lind-wasm}
ENARX=${ENARX:-$HOME/enarx/target/release/enarx}
OUT=$HERE/results/$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
echo "system,test,iters,total_ns,ns_per_op,skip_reason" >"$OUT/micro.csv"

# TriSeal takes its argv from Enarx.toml: write one per invocation.
triseal_toml() {
  local toml=$OUT/Enarx.$1.toml args
  args=$(printf ', "%s"' "$SYS_N" "$PROC_N" "$@")
  sed "s|^args = .*|args = [\"/microbench\"$args]|" \
    "$HERE/triseal/build/Enarx.microbench.toml" >"$toml"
  echo "$toml"
}

{
  date
  uname -r
  grep -m1 "model name" /proc/cpuinfo
  echo "governor $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)" \
       "no_turbo $(cat /sys/devices/system/cpu/intel_pstate/no_turbo)"
  echo "iters syscall=$SYS_N proc=$PROC_N"
  gramine-sgx --version 2>&1 | head -1
  grep -E "^(enclave_size|edmm_enable|max_threads) " "$HERE/gramine/microbench.manifest"
  docker image inspect occlum/occlum:latest-ubuntu20.04 --format "occlum image {{.Id}} {{.Created}}"
  jq -c .resource_limits "$HERE/occlum/inst/Occlum.json"
  echo "enarx $(stat -c %y "$ENARX") $(git -C "$(dirname "$ENARX")/../.." log -1 --format=%h)" \
       "dirty=$(git -C "$(dirname "$ENARX")/../.." status --short -uno | wc -l)"
  echo "lind-wasm $(git -C "$LIND" log -1 --format=%h)"
} >"$OUT/env.txt" 2>&1

# run <system> <cmd...>: append the output to <system>.log and its @mb lines
# to micro.csv. A hung keep is killed after RUN_TIMEOUT seconds.
run() {
  local sys=$1 tmp
  shift
  echo "== $sys $*" >&2
  tmp=$(mktemp)
  timeout "${RUN_TIMEOUT:-3600}" "$@" >"$tmp" 2>&1
  echo "rc=$?" >>"$tmp"
  cat "$tmp" >>"$OUT/$sys.log"
  awk -v s="$sys" '$1 == "@mb" {
    if ($3 == "skip") { r = $0; sub(/^@mb [^ ]+ skip /, "", r); print s "," $2 ",,,," r }
    else print s "," $2 "," $3 "," $4 "," $5 ","
  }' "$tmp" >>"$OUT/micro.csv"
  rm -f "$tmp"
}

for sys in $SYSTEMS; do
  case $sys in
    native)
      # /dev/shm keeps the file tests in memory, like the other systems.
      run native bash -c "cd '$HERE/gramine/app' && MB_FILE=/dev/shm/microbench.dat ./microbench $SYS_N $PROC_N" ;;
    gramine-direct)
      run gramine-direct bash -c "cd '$HERE/gramine' && gramine-direct microbench $SYS_N $PROC_N" ;;
    gramine-sgx)
      run gramine-sgx bash -c "cd '$HERE/gramine' && gramine-sgx microbench $SYS_N $PROC_N" ;;
    triseal-sgx)
      # Exited cages are not reclaimed (exec hits ENOMEM after ~34 cages),
      # so every process test gets a keep of its own.
      for group in "getpid clock_gettime pipe_rw pipe_pingpong file_write_64 file_write_4k file_read_64 file_read_4k" \
                   fork_exit fork_exec spawn_exec; do
        # shellcheck disable=SC2086
        run triseal-sgx env ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$(triseal_toml $group)" \
          "$ENARX" run "$LIND/lindfs/grates/imfs-grate.cwasm"
      done ;;
    occlum-sgx)
      run occlum-sgx "$HERE/occlum/run-docker.sh" bash -c \
        "cd inst && occlum run /bin/microbench $SYS_N $PROC_N" ;;
    *)
      echo "unknown system: $sys" >&2 ;;
  esac
done

# ns/op table: one row per test, one column per system.
python3 - "$OUT/micro.csv" <<'PY'
import csv, sys
rows = list(csv.DictReader(open(sys.argv[1])))
systems = list(dict.fromkeys(r["system"] for r in rows))
tests = list(dict.fromkeys(r["test"] for r in rows))
cell = {(r["system"], r["test"]): r["ns_per_op"] or "skip" for r in rows}
print(f"{'ns/op':15s}" + "".join(f"{s:>16s}" for s in systems))
for t in tests:
    print(f"{t:15s}" + "".join(f"{cell.get((s, t), '-'):>16s}" for s in systems))
PY
echo "results: $OUT"
