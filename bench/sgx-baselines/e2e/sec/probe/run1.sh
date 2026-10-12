#!/bin/bash
# Run one probe attempt in its own keep under the TriSeal SHM stack and print
# the keep's return code, which encodes the verdict (see probe.c):
#   rc 0 = SUCCEEDED (hole)   rc 1 = DENIED (clean)   rc 101/other = FAULTED
# Usage: run1.sh <attest|noattest> <cmd> [t] [addr]
set -uo pipefail
BENCH=$HOME/sgx-bench; E=$BENCH/e2e
ENARX=$E/shm/TriSeal/target/release/enarx
INTOTO=$BENCH/triseal/grates/attest/intoto-grate.cwasm
IMFS=$BENCH/triseal/grates/attest/imfs-grate.cwasm
MKPRELOAD=$BENCH/grates-attest/tools/mkpreload/target/release/mkpreload
PROBE=$BENCH/triseal/sec/probe.cwasm
mode=$1; shift
cmd=${1:-self}; t=${2:-0}; addr=${3:-0x10000}
W=/tmp/probe1.$$; rm -rf "$W"; mkdir -p "$W"
BUNDLE=/dev/shm/probe1.$$.prld; DUMP=/dev/shm/probe1.$$.dump
"$MKPRELOAD" "$BUNDLE" "$PROBE" >"$W/mk.log" 2>&1 || { tail -2 "$W/mk.log"; exit 3; }
pargs="\"$PROBE\", \"$cmd\""
[ "$t" != 0 ] && pargs="$pargs, \"$t\""
[ "$cmd" = readcage ] || [ "$cmd" = writecage ] && pargs="$pargs, \"$addr\""
if [ "$mode" = attest ]; then
  top=$INTOTO; flags='"--preload-shm", "--attest", '
else
  top=$IMFS; flags='"--preload-shm", '
fi
{
  if [ "$mode" = attest ]; then
    printf 'args = ["%s", "--preload-shm", "--attest", %s]\n' "$IMFS" "$pargs"
  else
    printf 'args = ["%s", "--preload-shm", %s]\n' "$IMFS" "$pargs"
  fi
  printf '\n[env]\nDUMPS = ""\n\n'
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((64<<20))
} >"$W/t.toml"
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/t.toml" \
  timeout 180 "$ENARX" run "$top" >"$W/out" 2>&1
rc=$?
rm -f "$BUNDLE" "$DUMP"
verdict=FAULTED
[ "$rc" = 0 ] && verdict=SUCCEEDED
[ "$rc" = 1 ] && verdict=DENIED
[ "$rc" = 124 ] && verdict=TIMEOUT
echo "RESULT mode=$mode cmd=$cmd t=$t addr=$addr keep_rc=$rc verdict=$verdict"
grep -E "^\[probe\]" "$W/out" 2>/dev/null | head
grep -iE "panic|GeneralProtection|trap|Unexpected|epoch" "$W/out" 2>/dev/null | grep -v "^\[fork" | head -2
