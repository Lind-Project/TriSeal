#!/bin/bash
# Rerun every benchmark against the current TriSeal runtime, one directory
# per category:
#   1-microbench/       run-micro.sh: all systems, syscall and process tests
#   2-fs-microbench/    file-compare.sh: file I/O times and host syscalls
#   3-e2e-cold-warm/    e2e/coldwarm.sh: Lua build, cold vs warm start
#   4-e2e-ecall-ocall/  e2e/counts.sh: enclave transitions during the build
# Usage: ./rerun-all.sh [e2e runs]   -> results/rerun-<timestamp>/
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
N=${1:-3}
R=${R:-$HERE/results/rerun-$(date +%Y%m%d-%H%M%S)}
E=${E:-$HOME/sgx-bench/e2e}
export ENARX=${ENARX:-$E/ts/TriSeal36/target/release/enarx}
mkdir -p "$R"

{
  echo "start $(date)"
  uname -r
  grep -m1 "model name" /proc/cpuinfo
  echo "governor $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)"
  echo "no_turbo $(cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null)"
  echo "enarx $ENARX $(stat -c %y "$ENARX")"
  echo "enarx (stats) $E/ts/TriSeal36s/target/release/enarx"
  echo "lind-wasm $(git -C "$E/lw-opt" log -1 --format='%h %s')"
  git -C "$E/lw-opt" status --short | sed 's/^/  /'
  echo "imfs grate $(git -C "$HOME/lind-wasm-example-grates-imfs-opt" log -1 --format='%h %s')"
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
step 2-fs-microbench "$HERE/file-compare.sh"
step 3-e2e-cold-warm "$HERE/e2e/coldwarm.sh" "$N"
step 4-e2e-ecall-ocall "$HERE/e2e/counts.sh"
echo "end $(date)" >>"$R/env.txt"
echo "results: $R"
