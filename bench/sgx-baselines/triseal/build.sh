#!/bin/bash
# Build <app>.c and its exec targets with the lind toolchain and write an
# Enarx.toml that boots them through the imfs grate (the grate itself must
# be a static build; the apps it runs may be dynamic).
# Usage: ./build.sh <app> [args...]   e.g. ./build.sh microbench 100000 20
# Knobs (env): STATIC=1 builds the apps statically too.
set -euo pipefail
APP=${1:-hello}
shift || true
LIND=${LIND:-$HOME/lind-wasm}
FLAGS=()
[ "${STATIC:-0}" = 1 ] && FLAGS=(-s)
cd "$(dirname "$0")"
mkdir -p build
for f in $(printf '%s\n' "$APP" hello nop | sort -u); do
  cp ../src/$f.c build/
  "$LIND/scripts/lind_compile" "${FLAGS[@]}" build/$f.c >build/$f.log 2>&1 \
    || { echo "lind_compile $f.c failed, see build/$f.log" >&2; exit 1; }
done
OUT=$PWD/build
ARGS=""
for a in "$@"; do ARGS+=", \"$a\""; done
# execv() inside the keep only sees files preloaded into the imfs grate.
cat > build/Enarx.$APP.toml <<EOT
args = ["/$APP"$ARGS]

[env]
PRELOADS = "/$APP=$OUT/$APP.cwasm:/automated_tests/hello=$OUT/hello.cwasm:/automated_tests/nop=$OUT/nop.cwasm"
EOT
echo "built $APP; run with:"
echo "  ENARX_BACKEND=sgx ENARX_WASMCFGFILE=$OUT/Enarx.$APP.toml enarx run $LIND/lindfs/grates/imfs-grate.cwasm"
