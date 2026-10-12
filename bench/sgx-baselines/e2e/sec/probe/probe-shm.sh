#!/bin/bash
# Run the isolation probe (experiment E) as a build-step cage under the TriSeal
# SHM/attest stack: the in-toto grate (which holds the signing key in its own
# cage) execs the IMFS grate, which execs the probe. So while the probe runs,
# the key-holding grate and the IMFS grate are live sibling cages it can try to
# reach. Prints the probe's DENIED/SUCCEEDED report.
#
# Usage: probe-shm.sh            PROBE=<probe.cwasm> overrides the program.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH=$HOME/sgx-bench
E=$BENCH/e2e
ENARX=${ENARX:-$E/shm/TriSeal/target/release/enarx}
INTOTO_GRATE=${INTOTO_GRATE:-$BENCH/triseal/grates/attest/intoto-grate.cwasm}
IMFS_GRATE=${IMFS_GRATE:-$BENCH/triseal/grates/attest/imfs-grate.cwasm}
MKPRELOAD=${MKPRELOAD:-$BENCH/grates-attest/tools/mkpreload/target/release/mkpreload}
PROBE=${PROBE:-$BENCH/triseal/sec/probe.cwasm}
W=${1:-$E/work-probe}
rm -rf "$W"; mkdir -p "$W"

BUNDLE=/dev/shm/probe.$$.prld
DUMP=/dev/shm/probe.$$.dump
"$MKPRELOAD" "$BUNDLE" "$PROBE" 2>"$W/mkpreload.log" || { tail -3 "$W/mkpreload.log"; exit 1; }

{
  printf 'args = ["%s", "--preload-shm", "--attest", "%s"]\n\n' "$IMFS_GRATE" "$PROBE"
  printf '[env]\nDUMPS = ""\n\n'
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((64 << 20))
} >"$W/run.toml"

LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/run.toml" \
  timeout 300 "$ENARX" run "$INTOTO_GRATE" >"$W/probe.out" 2>&1
rc=$?
rm -f "$BUNDLE" "$DUMP"
echo "=== probe run rc=$rc ==="
# The probe's own report (filter the runtime's fork/instrument noise).
grep -vE "^\[(fork|instrument)|failed to enable wasm cache" "$W/probe.out" | sed -n '/\[probe\] self/,/\[probe\] done/p'
echo "=== SUCCEEDED lines (holes), if any ==="
grep -c "SUCCEEDED" "$W/probe.out" 2>/dev/null
grep "SUCCEEDED" "$W/probe.out" 2>/dev/null | head
