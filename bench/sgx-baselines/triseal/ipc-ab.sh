#!/bin/bash
# Two IPC grate builds against each other on the pipe microbenchmarks
# (src/microbench.c in a TriSeal keep under the grate, run from its host path),
# alternating, N runs each: ns per call, median.
# Usage: triseal/ipc-ab.sh <grate A.cwasm> <grate B.cwasm>   (N=3, ITERS=100000,
#        ENARX=<runtime>, OUT=<dir>)
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
A=$1 B=$2
N=${N:-3}
ITERS=${ITERS:-100000}
ENARX=${ENARX:-$HOME/sgx-bench/e2e/t2/TriSeal/target/release/enarx}
OUT=${OUT:-$HERE/results/ipc-ab-$(date +%Y%m%d-%H%M%S)}
TESTS=${TESTS:-pipe_rw pipe_pingpong}
BUILD=$HERE/triseal/build
mkdir -p "$OUT"
{ echo "A $A $(sha256sum "$A" | cut -c1-16)"; echo "B $B $(sha256sum "$B" | cut -c1-16)"; echo "enarx $ENARX"; } >"$OUT/setup.txt"
toml=$OUT/microbench.toml
args=$(printf ', "%s"' "$ITERS" 1 $TESTS)
printf 'args = ["%s/microbench.cwasm"%s]\n' "$BUILD" "$args" >"$toml"
for i in $(seq "$N"); do
  for v in A B; do
    g=$A; [ $v = B ] && g=$B
    LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$toml" timeout 900 "$ENARX" run "$g" \
      >"$OUT/$v.$i.log" 2>&1
    echo "== $v $i: $(grep -c '^@mb' "$OUT/$v.$i.log") results" >&2
  done
done
python3 - "$OUT" "$N" $TESTS <<'PY'
import re, statistics, sys
out, n, tests = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
res = {}
for v in "AB":
    for i in range(1, n + 1):
        for m in re.finditer(r"^@mb (\S+) \d+ \d+ ([0-9.]+)$", open(f"{out}/{v}.{i}.log", errors="replace").read(), re.M):
            res.setdefault((v, m.group(1)), []).append(float(m.group(2)))
print(f"{'test':18} {'A ns':>9} {'B ns':>9} {'B-A':>8}")
for t in tests:
    a, b = res.get(("A", t)), res.get(("B", t))
    if a and b:
        ma, mb = statistics.median(a), statistics.median(b)
        print(f"{t:18} {ma:9.0f} {mb:9.0f} {mb - ma:8.0f}   A {a}  B {b}")
PY
