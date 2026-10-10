#!/bin/bash
# Lua build on TriSeal with the triseal-forkexec-opt runtime (64 GB enclave)
# and the versioned imfs grate, 3 runs. Results in results/e2e-opt-<ts>/.
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$HERE/../results/e2e-opt-$(date +%Y%m%d-%H%M%S); mkdir -p $OUT
for i in 1 2 3; do
  ENARX=$HERE/ts/TriSeal36/target/release/enarx IMFS_GRATE=$HOME/sgx-bench/triseal/grates/imfs-grate-clock.cwasm \
    $HERE/lua-triseal.sh $HERE/work-lua-opt > $OUT/triseal-opt.$i.out 2>&1
  grep -h "^wall\|^Lua" $OUT/triseal-opt.$i.out | tr "\n" " "; echo
done
echo "results: $OUT"
