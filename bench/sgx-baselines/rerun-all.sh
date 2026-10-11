#!/bin/bash
# Rerun every benchmark against the current TriSeal, one directory per
# category:
#   1-microbench/       run-micro.sh: syscall, pipe, file and process tests
#                       (TriSeal: pipes under the IPC grate, the rest under
#                       the IMFS grate)
#   2-fs-microbench/    file-compare.sh: file I/O per file system, host
#                       syscalls per call (TriSeal: IMFS)
#   3-sfp/              e2e/sfp/run-all.sh: oggenc.c and gcc.c on native,
#                       TriSeal, Gramine and Occlum, cold vs warm, and what
#                       each build asks of the host
#   4-e2e-cold-warm/    e2e/coldwarm.sh: Lua build, cold vs warm start
#   5-e2e-ecall-ocall/  e2e/after-start.sh: enclave transitions from the
#                       start of the Lua build to its end
# Usage: ./rerun-all.sh [e2e runs]   -> results/rerun-<timestamp>/
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
N=${1:-3}
R=${R:-$HERE/results/rerun-$(date +%Y%m%d-%H%M%S)}
E=${E:-$HOME/sgx-bench/e2e}
# TriSeal open-futex (stats built in, ENARX_SGX_STATS) on lind-wasm
# enarx-7-13 (e2e/r3). Grates: IMFS and IPC keeping their cage id, IMFS
# reserving its chunk arena before staging; in-toto as before. Toolchain:
# cc1, as and ld without Asyncify (e2e/root-na).
export ENARX=${ENARX:-$E/r3/TriSeal/target/release/enarx}
export ENARX_STATS=${ENARX_STATS:-$ENARX}
export IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/r3/imfs-grate.cwasm}
export IPC_GRATE=${IPC_GRATE:-$HERE/triseal/grates/r3/ipc-grate.cwasm}
export INTOTO_GRATE=${INTOTO_GRATE:-$HERE/triseal/grates/r3/intoto-grate.cwasm}
export ROOT=${ROOT:-$E/root-na}
mkdir -p "$R"

sum() { sha256sum "$1" | cut -c1-16; }
{
  echo "start $(date)"
  uname -r
  grep -m1 "model name" /proc/cpuinfo
  echo "governor $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)"
  echo "no_turbo $(cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null)"
  echo "enarx $ENARX $(stat -c %y "$ENARX") $(sum "$ENARX")"
  echo "enarx (stats) $ENARX_STATS"
  t=$(dirname "$(dirname "$(dirname "$ENARX")")")
  echo "TriSeal source $(cat "$t/SOURCE_COMMIT" 2>/dev/null)"
  echo "lind-wasm $(readlink -f "$t/lind-wasm")"
  for g in "$IMFS_GRATE" "$IPC_GRATE" "$INTOTO_GRATE"; do
    echo "grate $g $(stat -c %y "$g") $(sum "$g")"
  done
  echo "toolchain $ROOT"
  for b in make gcc cc1 as ld; do echo "  $b $(sum "$ROOT/usr/local/bin/$b")"; done
  echo "gramine $(gramine-sgx --version 2>/dev/null || dpkg-query -W gramine 2>/dev/null)"
  echo "gramine (stats) $HOME/sgx-bench/gramine-dbg"
  echo "occlum $(docker image inspect occlum/occlum:latest-ubuntu20.04 --format '{{.Id}}' 2>/dev/null)"
} >"$R/env.txt"

step() {
  local name=$1
  shift
  echo "== $name $(date +%T)" | tee -a "$R/env.txt" >&2
  OUT=$R/$name "$@" >"$R/$name.out" 2>&1
  echo "   rc=$? $(date +%T)" | tee -a "$R/env.txt" >&2
}
step 1-microbench "$HERE/run-micro.sh" 100000 20
step 2-fs-microbench "$HERE/file-compare.sh" native gramine-tmpfs gramine-allowed \
  gramine-encrypted gramine-trusted triseal-imfs occlum-ramfs occlum-sefs
step 3-sfp env N="$N" "$HERE/e2e/sfp/run-all.sh" "$R/3-sfp"
step 4-e2e-cold-warm "$HERE/e2e/coldwarm.sh" "$N"
step 5-e2e-ecall-ocall "$HERE/e2e/after-start.sh"
echo "end $(date)" >>"$R/env.txt"
echo "results: $R"
