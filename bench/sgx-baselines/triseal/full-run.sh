#!/bin/bash
# Run one program in TriSeal's full setup, as e2e/lua-triseal-shm.sh runs the
# Lua build: the host packs the program and every module it execs into a
# bundle in shared memory; the in-toto grate runs the IMFS grate, which copies
# the bundle in, has it attested, marks the start of the program (the shim
# seals) and runs it; at exit the in-toto grate signs the products (none
# here), the runtime signs run-end, and the evidence goes to the dump
# region, which is checked up to the quote (e2e/verify-attest.py).
# Usage: triseal/full-run.sh <workdir> <imfs_path=host_path>... -- <program> [args...]
#   The program's output, then the check, go to stdout; the exit status is
#   the keep's, or 1 if the evidence does not verify.
#   ENVS     TOML lines for [env], each ending in a newline (e.g.
#            $'MB_FILE = "/tmp/mb.dat"\n')
#   TRACE    a command prefix for enarx (e.g. strace -f -c -o <file>)
# ENARX, INTOTO_GRATE, IMFS_GRATE and MKPRELOAD as in e2e/lua-triseal-shm.sh.
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
ENARX=${ENARX:-$E/s73b/TriSeal/target/release/enarx}
INTOTO_GRATE=${INTOTO_GRATE:-$HERE/triseal/grates/ev/intoto-grate.cwasm}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/ev/imfs-grate.cwasm}
MKPRELOAD=${MKPRELOAD:-$HOME/sgx-bench/grates-attest/tools/mkpreload/target/release/mkpreload}
W=$1
shift
entries=()
while [ $# -gt 0 ] && [ "$1" != -- ]; do entries+=("$1"); shift; done
shift
mkdir -p "$W"
rm -rf "$W/dump" "$W/dump.prld"

BUNDLE=/dev/shm/full-run.$$.prld
DUMP=/dev/shm/full-run.$$.dump
"$MKPRELOAD" "$BUNDLE" "${entries[@]}" 2>"$W/mkpreload.log" || { tail -3 "$W/mkpreload.log"; exit 1; }
{
  printf 'args = ['
  sep=""
  for a in "$IMFS_GRATE" --preload-shm --attest --dump-shm "$@"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nDUMPS = ""\n%s\n' "${ENVS:-}"
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((16 << 20))
} >"$W/run.toml"

ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/run.toml" \
  timeout "${RUN_TIMEOUT:-3600}" ${TRACE:-} "$ENARX" run "$INTOTO_GRATE" >"$W/run.log" 2>&1
rc=$?
grep -vE "^\[(fork|instrument)" "$W/run.log"
if [ -s "$DUMP" ]; then
  cp "$DUMP" "$W/dump.prld"
  "$MKPRELOAD" unpack "$W/dump.prld" "$W/dump" >/dev/null 2>&1
fi
if [ -f "$W/dump/evidence/evidence.json" ] && python3 "$HERE/e2e/verify-attest.py" "$W/dump" "$BUNDLE"; then
  :
else
  echo "evidence NOT verified"
  [ $rc = 0 ] && rc=1
fi
rm -f "$BUNDLE" "$DUMP"
echo "keep rc=$rc"
exit $rc
