#!/bin/bash
# Rerun every benchmark against the current TriSeal, one directory per
# category:
#   1-microbench/       run-micro.sh: syscall, pipe, file and process tests
#                       (TriSeal: pipes under the IPC grate, the rest under
#                       the IMFS grate; Gramine: file tests on its encrypted
#                       mount)
#   2-fs-microbench/    file-compare.sh: file I/O, host syscalls per call
#                       (TriSeal: IMFS; Gramine: its encrypted mount)
# TriSeal runs in its full setup only (TRISEAL_SETUPS=shm, the default): the
# inputs in shared memory, attested by the in-toto grate, the shim sealed
# while the workload runs, the outputs signed and the evidence checked up to
# the quote after every run (triseal/full-run.sh, e2e/lua-triseal-shm.sh,
# e2e/sfp/triseal-shm.sh). TRISEAL_SETUPS="preloads shm" adds the setup that
# reads the inputs from host files and makes no evidence (triseal-sgx,
# triseal-imfs, e2e/lua-triseal.sh, e2e/sfp/triseal.sh).
#   3-sfp/              e2e/sfp/run-all.sh: oggenc.c and gcc.c on native,
#                       TriSeal, Gramine and Occlum, cold vs warm, and what
#                       each build asks of the host
#   4-e2e-cold-warm/    e2e/coldwarm.sh: Lua build, cold vs warm start
#                       (native, TriSeal, Gramine, Occlum SGX1 and EDMM)
#   5-e2e-ecall-ocall/  e2e/after-start.sh: enclave transitions from the
#                       start of the Lua build to its end
# Usage: ./rerun-all.sh [e2e runs]   -> results/rerun-<timestamp>/
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
N=${1:-3}
R=${R:-$HERE/results/rerun-$(date +%Y%m%d-%H%M%S)}
E=${E:-$HOME/sgx-bench/e2e}
# TriSeal triseal-t2-t7-t3 (open-futex a442cc90 + the shim sealed once the
# build starts + a compiled-in module allowlist + the evidence chain anchored
# to the quote; stats built in, ENARX_SGX_STATS) on lind-wasm lw-ev
# (enarx-7-13 + module admission + LIND_EVIDENCE), built with the
# evaluation's allowlist plus the benchmark programs (e2e/s73b). Grates: IMFS
# and in-toto signing the outputs and handing over the evidence (ev); IPC
# keeping its cage id (r3). Toolchain: cc1, as and ld without Asyncify
# (e2e/root-na).
export ENARX=${ENARX:-$E/s73b/TriSeal/target/release/enarx}
export ENARX_STATS=${ENARX_STATS:-$ENARX}
export IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/ev/imfs-grate.cwasm}
export IPC_GRATE=${IPC_GRATE:-$HERE/triseal/grates/r3/ipc-grate.cwasm}
export INTOTO_GRATE=${INTOTO_GRATE:-$HERE/triseal/grates/ev/intoto-grate.cwasm}
export ROOT=${ROOT:-$E/root-na}
export TRISEAL_SETUPS=${TRISEAL_SETUPS:-shm}
setup() { [[ " $TRISEAL_SETUPS " == *" $1 "* ]]; }
TS_MICRO=""; TS_FS=""; TS_BUILD=""
setup preloads && { TS_MICRO+=" triseal-sgx"; TS_FS+=" triseal-imfs"; TS_BUILD+=" triseal"; }
setup shm && { TS_MICRO+=" triseal-shm"; TS_FS+=" triseal-shm"; TS_BUILD+=" triseal-shm"; }
# The Lua build cold vs warm on these systems (e2e/coldwarm.sh SYSTEMS).
COLDWARM_SYSTEMS=${COLDWARM_SYSTEMS:-native$TS_BUILD gramine occlum occlum-edmm}
SFP_SYSTEMS=${SFP_SYSTEMS:-native$TS_BUILD gramine-encrypted gramine-allowed occlum}
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
  echo "lind-wasm $(readlink -f "$t/../lind-wasm")"
  [ -f "$t/../allowlist.txt" ] && echo "module allowlist $(readlink -f "$t/../allowlist.txt") $(sum "$t/../allowlist.txt")"
  for g in "$IMFS_GRATE" "$IPC_GRATE" "$INTOTO_GRATE"; do
    echo "grate $g $(stat -c %y "$g") $(sum "$g")"
  done
  echo "TriSeal setups: $TRISEAL_SETUPS"
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
step 1-microbench "$HERE/run-micro.sh" 100000 20 native gramine-direct gramine-sgx $TS_MICRO occlum-sgx
step 2-fs-microbench "$HERE/file-compare.sh" native gramine-encrypted $TS_FS \
  occlum-ramfs occlum-sefs
step 3-sfp env N="$N" SYSTEMS="$SFP_SYSTEMS" "$HERE/e2e/sfp/run-all.sh" "$R/3-sfp"
step 4-e2e-cold-warm env SYSTEMS="$COLDWARM_SYSTEMS" "$HERE/e2e/coldwarm.sh" "$N"
step 5-e2e-ecall-ocall "$HERE/e2e/after-start.sh"
echo "end $(date)" >>"$R/env.txt"
echo "results: $R"
