#!/bin/bash
# Benchmark: the time-source canaries. The canary project's `make time`
# (tcanary + libcanary.a, canary/proj/Makefile) three times in each
# configuration, the configurations interleaved, a few seconds apart:
#   native        native gcc, as anyone would build it
#   native-sde    the same with SOURCE_DATE_EPOCH=$EPOCH in the environment
#   triseal       TriSeal's full setup, no logical clock
#   triseal-sde   the same with SOURCE_DATE_EPOCH declared in the invocation
#   triseal-clock the logical clock above make, epoch $EPOCH declared, and
#                 the IMFS grate that starts file times at that epoch
# No configuration changes the project: same Makefile, same flags (ar U, the
# non-deterministic mode, explicitly). canary/report.py tabulates.
# Usage: canary/time.sh [results dir]   (EPOCH, default 1700000000; RUNS, 3)
set -u
K=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
B=$HOME/sgx-bench; E=$B/e2e; C=$E/clang
R=${1:-$B/results/canary-time-$(date +%Y%m%d-%H%M%S)}; mkdir -p "$R"
EPOCH=${EPOCH:-1700000000}
{ echo "start $(date)"; echo "enarx $K/enarx-default $(sha256sum < $K/enarx-default | cut -c1-16)"
  echo "probe (TriSeal) $(sha256sum < $K/ebc/probe | cut -c1-16), probe (native) $(sha256sum < $K/native-bin/probe | cut -c1-16)"
  echo "epoch $EPOCH"; } > "$R/env.txt"
one() {  # <name> <driver> [env...]
  local n=$1 d=$2; shift 2
  env TARGET=time "$@" bash "$K/$d.sh" "$R/$n" > "$R/$n.out" 2>&1
  printf '%-16s %s\n' "$n" "$(grep -E '^wall' "$R/$n.out") $(grep -c '^evidence verified' "$R/$n.out") verified" | tee -a "$R/summary.txt"
  sleep 5
}
for i in $(seq ${RUNS:-3}); do
  one native-$i native
  one native-sde-$i native SOURCE_DATE_EPOCH=$EPOCH
  one triseal-$i triseal
  one triseal-sde-$i triseal GUEST_ENV="SOURCE_DATE_EPOCH = \"$EPOCH\""
  one triseal-clock-$i triseal IMFS_GRATE=$C/ebc/imfs-grate-clock.cwasm CLOCK=$C/ebc/logical-clock-grate.cwasm \
    GUEST_ENV="LOGICAL_CLOCK_EPOCH = \"$EPOCH\""
done
echo "end $(date)" >> "$R/env.txt"
python3 "$K/report.py" "$R" | tee "$R/report.md"
echo "TIME_DONE $R"
