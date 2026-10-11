#!/bin/bash
# What each single-file build asks of the host, from its start to its end:
#   TriSeal  the stats runtime: counters at the IMFS phase mark (make starts)
#            and at exit (enclave entries, exceptions, ocalls, and in the
#            enclave syscalls through the direct entry and pages committed),
#            in both setups: triseal-preloads (triseal.sh, IMFS stages host
#            files and writes the output back through the host at exit) and
#            triseal-shm (triseal-shm.sh, preloads from shared memory,
#            attested, the output dumped to shared memory)
#   Gramine  (encrypted files) sgx.enable_stats, build minus probe
#   all three, Occlum included: host syscalls of the whole TEE process tree
#            (strace -f -c), build minus probe (`make --version`). Occlum
#            has no counters of its own, so this is its ocall measure.
# Usage: e2e/sfp/after-start.sh     (OUT=<dir>; one subdirectory per program)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
BENCH=$(cd "$HERE/../.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
OUT=${OUT:-$E/sfp-after-start}
ENARX_STATS=${ENARX_STATS:-$E/ze/TriSeal-s/target/release/enarx}
GRAMINE_DBG=${GRAMINE_DBG:-$HOME/sgx-bench/gramine-dbg}
genclave() { case $1 in gcc) echo "${GRAMINE_ENCLAVE_gcc:-4G}" ;; *) echo "${GRAMINE_ENCLAVE:-1G}" ;; esac; }
PROGS=${PROGS:-${SFP_PROGS[*]}}
for p in $PROGS; do
  o=$OUT/$p
  mkdir -p "$o"
  echo "== $p triseal stats $(date +%T)" >&2
  ENARX_SGX_STATS=y ENARX=$ENARX_STATS "$HERE/triseal.sh" "$p" "$E/work-sfp-as-triseal" >"$o/triseal-preloads.out" 2>&1
  cp "$E/work-sfp-as-triseal/build.log" "$o/triseal-preloads.log"
  ENARX_SGX_STATS=y ENARX=$ENARX_STATS "$HERE/triseal-shm.sh" "$p" "$E/work-sfp-as-triseal-shm" >"$o/triseal-shm.out" 2>&1
  cp "$E/work-sfp-as-triseal-shm/build.log" "$o/triseal-shm.log"
  for kind in build probe; do
    probe=0
    [ $kind = probe ] && probe=1
    echo "== $p gramine stats $kind $(date +%T)" >&2
    PATH=$GRAMINE_DBG/bin:$PATH \
      PYTHONPATH=$(echo "$GRAMINE_DBG"/lib/python3*/site-packages) \
      STATS=1 PROBE=$probe MODE=encrypted ENCLAVE_SIZE=$(genclave "$p") \
      "$HERE/gramine.sh" "$p" "$E/work-sfp-as-gramine" >"$o/gramine.$kind.out" 2>&1
    cp "$E/work-sfp-as-gramine.log" "$o/gramine.$kind.log"
    echo "== $p strace $kind $(date +%T)" >&2
    TRACE="strace -f -c -o $o/strace.triseal.$kind.txt" PROBE=$probe \
      "$HERE/triseal.sh" "$p" "$E/work-sfp-as-triseal" >"$o/strace.triseal.$kind.out" 2>&1
    TRACE="strace -f -c -o $o/strace.triseal-shm.$kind.txt" PROBE=$probe \
      "$HERE/triseal-shm.sh" "$p" "$E/work-sfp-as-triseal-shm" >"$o/strace.triseal-shm.$kind.out" 2>&1
    TRACE="strace -f -c -o $o/strace.gramine-encrypted.$kind.txt" PROBE=$probe MODE=encrypted \
      ENCLAVE_SIZE=$(genclave "$p") "$HERE/gramine.sh" "$p" "$E/work-sfp-as-gramine" \
      >"$o/strace.gramine-encrypted.$kind.out" 2>&1
    STRACE=1 PROBE=$probe "$HERE/occlum.sh" "$p" >"$o/strace.occlum.$kind.out" 2>&1
    cp "$BENCH/occlum/sfp/strace.$p.$kind.txt" "$o/strace.occlum.$kind.txt"
  done
  python3 "$HERE/../after-start.py" "$o" >"$o/summary.txt"
  python3 "$HERE/strace-summary.py" "$o" >>"$o/summary.txt"
  cat "$o/summary.txt"
done
