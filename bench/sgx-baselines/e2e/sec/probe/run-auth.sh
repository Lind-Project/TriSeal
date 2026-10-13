#!/bin/bash
# Run one handler-table authorization probe (probe-auth.c) in its own keep and
# print the verdict the probe reports (SUCCEEDED = a hole, DENIED). The keep's
# return code does not carry the probe's exit code, so a run that prints no
# verdict is FAULTED (or TIMEOUT).
#
# Usage: run-auth.sh <r3|auth> <bare|imfs> <cmd> [arg]
#   r3    TriSeal r3 runtime (lind enarx-7-13 27f710a09, no check)
#   auth  same runtime on lind threei-handler-auth (e2e/auth)
#   bare  the probe is the first cage
#   imfs  the probe runs under the IMFS grate (SHM preload), as a build step does
set -uo pipefail
BENCH=$HOME/sgx-bench; E=$BENCH/e2e
rt=$1; stack=$2; cmd=$3; arg=${4:-}
case $rt in
  r3) ENARX=$E/r3/TriSeal/target/release/enarx ;;
  auth) ENARX=$E/auth/TriSeal/target/release/enarx ;;
  *) echo "runtime: r3 or auth"; exit 2 ;;
esac
IMFS=$BENCH/triseal/grates/r3/imfs-grate.cwasm
MKPRELOAD=$BENCH/grates-attest/tools/mkpreload/target/release/mkpreload
PROBE=$BENCH/triseal/sec/auth/probe-auth.cwasm
W=$E/work-auth/$rt-$stack-$cmd; rm -rf "$W"; mkdir -p "$W"
pargs="\"$cmd\""
[ -n "$arg" ] && pargs="$pargs, \"$arg\""
if [ "$stack" = imfs ]; then
  BUNDLE=/dev/shm/auth.$$.prld; DUMP=/dev/shm/auth.$$.dump
  "$MKPRELOAD" "$BUNDLE" "$PROBE" >"$W/mk.log" 2>&1 || { tail -2 "$W/mk.log"; exit 3; }
  {
    # IMFS is the main module: its args start at argv[1].
    printf 'args = ["--preload-shm", "%s", %s]\n' "$PROBE" "$pargs"
    printf '\n[env]\nDUMPS = ""\n\n'
    printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((64 << 20))
  } >"$W/t.toml"
  top=$IMFS
else
  # Bare: args start at argv[1].
  printf 'args = [%s]\n' "$pargs" >"$W/t.toml"
  top=$PROBE
fi
ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/t.toml" timeout 180 "$ENARX" run "$top" >"$W/out" 2>&1
rc=$?
[ "$stack" = imfs ] && rm -f "$BUNDLE" "$DUMP"
verdict=$(grep -oE "(SUCCEEDED|DENIED)" "$W/out" 2>/dev/null | tail -1)
[ -z "$verdict" ] && verdict=FAULTED
[ "$rc" = 124 ] && verdict=TIMEOUT
echo "RESULT rt=$rt stack=$stack cmd=$cmd keep_rc=$rc verdict=$verdict"
grep -E "^\[auth\]" "$W/out" 2>/dev/null | head
grep -iE "panic|GeneralProtection|trap|Unexpected" "$W/out" 2>/dev/null | grep -v "^\[fork" | head -2
