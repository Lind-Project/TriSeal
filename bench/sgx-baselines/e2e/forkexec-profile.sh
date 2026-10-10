#!/bin/bash
# Time the phases of fork+exec inside a TriSeal keep. make execs <prog>
# --version N times under the imfs grate; every stderr line is timestamped on
# the host, and forkexec-phases.py turns the [instrument] markers into mean
# seconds per phase (fork, memory copy, module read via 3i, deserialize,
# asyncify, instantiate).
# Usage: e2e/forkexec-profile.sh <prog> [N]   e.g. e2e/forkexec-profile.sh cc1 10
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
BIN=$E/root/usr/local/bin
ENARX=${ENARX:-$E/ts/TriSeal/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/imfs-grate-opt.cwasm}
P=$1
N=${2:-10}
D=$E/forkexec-profile
mkdir -p "$D"
{
  printf 'all:\n'
  for _ in $(seq "$N"); do printf '\t%s --version\n' "$BIN/$P"; done
} >"$D/Makefile.$P"
printf 'args = ["%s", "-f", "%s"]\n\n[env]\nPRELOADS = "%s:%s:%s"\n' \
  "$BIN/make" "$D/Makefile.$P" "$BIN/make" "$BIN/$P" "$D/Makefile.$P" >"$D/$P.toml"
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$D/$P.toml" \
  timeout 1800 "$ENARX" run "$IMFS_GRATE" 2>&1 |
  python3 -u -c 'import sys, time
t0 = time.monotonic()
for line in sys.stdin:
    sys.stdout.write(f"{time.monotonic() - t0:.4f} {line}")' >"$D/$P.ts.log"
python3 "$(dirname "$0")/forkexec-phases.py" "$D/$P.ts.log"
