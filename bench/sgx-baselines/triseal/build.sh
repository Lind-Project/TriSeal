#!/bin/bash
# Build <app>.c and hello.c with the lind toolchain (static; the runtime in
# the keep does not resolve the dynamic build's imports) and write an
# Enarx.toml that boots them through the imfs grate.
# Usage: ./build.sh <app>   e.g. ./build.sh forkexecv
set -euo pipefail
APP=${1:-hello}
LIND=${LIND:-$HOME/lind-wasm}
cd "$(dirname "$0")"
mkdir -p build
for f in $(printf '%s\n' "$APP" hello | sort -u); do
  cp ../src/$f.c build/
  "$LIND/scripts/lind_compile" -s build/$f.c >build/$f.log 2>&1 \
    || { echo "lind_compile $f.c failed, see build/$f.log" >&2; exit 1; }
done
OUT=$PWD/build
# execv() inside the keep only sees files preloaded into the imfs grate.
cat > build/Enarx.$APP.toml <<EOT
args = ["/$APP"]

[env]
PRELOADS = "/$APP=$OUT/$APP.cwasm:/automated_tests/hello=$OUT/hello.cwasm"
EOT
echo "built $APP; run with:"
echo "  ENARX_BACKEND=sgx ENARX_WASMCFGFILE=$OUT/Enarx.$APP.toml enarx run $LIND/lindfs/grates/imfs-grate.cwasm"
