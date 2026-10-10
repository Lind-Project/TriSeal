#!/bin/bash
# A/B: wall seconds of make exec'ing as/cc1 10 times, and microbench fork_exit x5.
# Usage: abtest.sh <label> <enarx>
E=~/sgx-bench/e2e; D=$E/execcost; G=${GRATE:-~/sgx-bench/triseal/grates/imfs-grate-opt.cwasm}
lab=$1; X=$2
for t in as cc1 fork; do
  s=$(date +%s.%N)
  LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE=$D/$t.toml timeout 1800 $X run $G > $D/ab.$lab.$t.log 2>&1
  e=$(date +%s.%N)
  extra=$(grep -h "^@mb" $D/ab.$lab.$t.log | awk '{printf " fork_exit=%.3fs/op", $5/1e9}')
  printf "%-10s %-5s wall %7.2f s%s\n" "$lab" "$t" "$(echo "$e - $s" | bc)" "$extra"
done
