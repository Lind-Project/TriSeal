#!/bin/bash
# Run the in-process microbenchmarks (src/microbench.c) on every system and
# save the results. Build first:
#   (cd gramine && make APP=microbench)
#   triseal/build.sh microbench
#   occlum/run-docker.sh ./build.sh microbench
# Usage: ./run-micro.sh [syscall_iters] [proc_iters] [system...]
#   systems: native gramine-direct gramine-sgx triseal-sgx triseal-shm
#   triseal-nogrates occlum-sgx (default: native, Gramine, triseal-sgx,
#   Occlum). triseal-sgx runs the pipe tests under the IPC grate and every
#   other test under the IMFS grate (one column); triseal-shm runs them in
#   TriSeal's full setup (triseal/full-run.sh: microbench and the programs
#   it execs in an attested bundle, the shim sealed while it runs, the
#   evidence checked up to the quote), pipes under the IPC grate that IMFS
#   runs; triseal-nogrates runs microbench as the keep's workload (host
#   files).
# ENARX / IMFS_GRATE / IPC_GRATE / INTOTO_GRATE pick TriSeal's runtime and
# grates.
# GRAMINE_MB_FILE is Gramine's scratch file for the file tests: by default on
# its encrypted mount (/enc, host files encrypted by Gramine); /tmp/... puts
# it on Gramine's tmpfs (in-enclave memory, private to a process).
# Writes results/<timestamp>/: micro.csv, one <system>.log each, env.txt.
set -uo pipefail
SYS_N=${1:-100000}
PROC_N=${2:-20}
shift 2 2>/dev/null || shift $#
SYSTEMS=${*:-native gramine-direct gramine-sgx triseal-sgx occlum-sgx}
HERE=$(cd "$(dirname "$0")" && pwd)
LIND=${LIND:-$HOME/lind-wasm}
ENARX=${ENARX:-$HOME/sgx-bench/e2e/ts/TriSeal36/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/imfs-grate-clock.cwasm}
IPC_GRATE=${IPC_GRATE:-$HERE/triseal/grates/ipc-grate.cwasm}
GRAMINE_MB_FILE=${GRAMINE_MB_FILE:-/enc/microbench.dat}
OUT=${OUT:-$HERE/results/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
echo "system,test,iters,total_ns,ns_per_op,skip_reason" >"$OUT/micro.csv"

# TriSeal takes its argv from Enarx.toml: write one per invocation.
triseal_toml() {
  local toml=$OUT/Enarx.$1.toml args
  # $1 names the file; the tests follow (none runs every test).
  shift
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
  echo "gramine file tests on $GRAMINE_MB_FILE"
  grep -E "^(enclave_size|edmm_enable|max_threads) " "$HERE/gramine/microbench.manifest"
  docker image inspect occlum/occlum:latest-ubuntu20.04 --format "occlum image {{.Id}} {{.Created}}"
  jq -c .resource_limits "$HERE/occlum/inst/Occlum.json"
  echo "enarx $ENARX $(stat -c %y "$ENARX")"
  echo "lind-wasm runtime $(git -C "$HOME/sgx-bench/e2e/lw-opt" log -1 --format='%h %s' 2>/dev/null)"
  echo "imfs grate $IMFS_GRATE $(stat -c %y "$IMFS_GRATE")"
  echo "ipc grate $IPC_GRATE $(stat -c %y "$IPC_GRATE")"
  echo "in-toto grate ${INTOTO_GRATE:-} $(stat -c %y "${INTOTO_GRATE:-/nonexistent}" 2>/dev/null)"
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
      # /dev/shm keeps the file tests in memory.
      run native bash -c "cd '$HERE/gramine/app' && MB_FILE=/dev/shm/microbench.dat ./microbench $SYS_N $PROC_N" ;;
    gramine-direct)
      run gramine-direct bash -c "cd '$HERE/gramine' && MB_FILE='$GRAMINE_MB_FILE' gramine-direct microbench $SYS_N $PROC_N" ;;
    gramine-sgx)
      run gramine-sgx bash -c "cd '$HERE/gramine' && MB_FILE='$GRAMINE_MB_FILE' gramine-sgx microbench $SYS_N $PROC_N" ;;
    triseal-sgx)
      # Files and processes under the IMFS grate, pipes under the IPC grate
      # (IMFS has no pipes), which runs microbench from its host path.
      run triseal-sgx env LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx \
        ENARX_WASMCFGFILE="$(triseal_toml imfs getpid clock_gettime file_lseek file_open_close \
          file_stat file_write_64 file_write_4k file_read_64 file_read_4k fork_exit fork_exec spawn_exec)" \
        "$ENARX" run "$IMFS_GRATE"
      printf 'args = ["%s/triseal/build/microbench.cwasm", "%s", "%s", "pipe_rw", "pipe_pingpong"]\n' \
        "$HERE" "$SYS_N" "$PROC_N" >"$OUT/Enarx.ipc.toml"
      run triseal-sgx env LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$OUT/Enarx.ipc.toml" \
        "$ENARX" run "$IPC_GRATE" ;;
    triseal-shm)
      # The full setup: one keep for files and processes under IMFS, and one
      # per pipe test under the IPC grate, which IMFS runs (IMFS has no
      # pipes). pipe_pingpong gets a keep of its own: the sealed shim
      # refuses the clock_nanosleep it reaches (it failed with EINTR before
      # the seal), and that ends the keep.
      B=$HERE/triseal/build
      progs=("/microbench=$B/microbench.cwasm" "/automated_tests/hello=$B/hello.cwasm"
             "/automated_tests/nop=$B/nop.cwasm")
      run triseal-shm env IMFS_GRATE="$IMFS_GRATE" "$HERE/triseal/full-run.sh" "$OUT/full-imfs" "${progs[@]}" -- \
        /microbench "$SYS_N" "$PROC_N" getpid clock_gettime file_lseek file_open_close \
        file_stat file_write_64 file_write_4k file_read_64 file_read_4k fork_exit fork_exec spawn_exec
      for t in pipe_rw pipe_pingpong; do
        run triseal-shm env IMFS_GRATE="$IMFS_GRATE" "$HERE/triseal/full-run.sh" "$OUT/full-$t" "${progs[@]}" "$IPC_GRATE" -- \
          "$IPC_GRATE" /microbench "$SYS_N" "$PROC_N" "$t"
      done ;;
    triseal-nogrates)
      # No imfs: files are host files, and exec targets are not preloaded,
      # so only the syscall, pipe and file tests run here.
      printf 'args = ["%s", "%s", %s]\n\n[env]\nMB_FILE = "/tmp/microbench.dat"\n' "$SYS_N" "$PROC_N" \
        '"getpid", "clock_gettime", "pipe_rw", "pipe_pingpong", "file_lseek", "file_open_close", "file_stat", "file_write_64", "file_write_4k", "file_read_64", "file_read_4k"' \
        >"$OUT/Enarx.nogrates.toml"
      run triseal-nogrates env ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$OUT/Enarx.nogrates.toml" \
        "$ENARX" run "$HERE/triseal/build/microbench.cwasm" ;;
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
