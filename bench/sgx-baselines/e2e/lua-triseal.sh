#!/bin/bash
# Build Lua 5.5.1 with its own Makefile inside a TriSeal SGX keep: make, gcc,
# cc1, as, ld, ar and ranlib all run as cages under the imfs grate, so every
# source, header, object and archive stays in enclave memory. lua and luac
# are dumped back to the host at exit and smoke-tested natively.
#
# Needs the staged tree from stage.sh under $E (default ~/sgx-bench/e2e).
# Usage: e2e/lua-triseal.sh [workdir]     (prints "wall <s>" for the keep)
#   INTOTO=1 wraps make in `in-toto-cli run`, signing a link over the
#   sources (materials) and lua/luac (products) with a key made in the keep.
#   PROBE=1 runs `make --version` with the same preloads instead, to measure
#   keep startup plus preloading on their own.
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
ENARX=${ENARX:-$HOME/enarx/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/imfs-grate-opt.cwasm}
CLI=${CLI:-$E/intoto/in-toto-cli.cwasm}
W=${1:-$E/work-lua}

rm -rf "$W"
mkdir -p "$W"
cp -r "$E/lua-5.5.1-src/." "$W/"
SRC=$W/src
CC="$BIN/gcc -std=gnu99 -B$BIN/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"

# Headers the build reads, from the host compiler with the same search path.
headers() {
  gcc -M -nostdinc -std=gnu99 -O2 -DLUA_USE_POSIX -isystem "$G/include" -isystem "$R/usr/include" "$SRC"/*.c |
    tr -d '\\' | tr ' ' '\n' | grep '^/' | grep -v "^$W/" | sort -u
}

files=(
  "$BIN"/{make,gcc,cc1,as,ld,ar,ranlib}
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
  "$SRC"/*.c "$SRC"/*.h "$SRC"/Makefile
)
# Absolute tool paths: nothing is on PATH inside the keep.
MAKE_CMD=("$BIN/make" -C "$SRC" all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=$BIN/ar rcu" "RANLIB=$BIN/ranlib")
DUMPS="$SRC/lua=$SRC/lua;$SRC/luac=$SRC/luac"

if [ "${PROBE:-0}" = 1 ]; then
  MAKE_CMD=("$BIN/make" --version)
fi

if [ "${INTOTO:-0}" = 1 ]; then
  KEY=$W/func.pk8
  # The key is made in its own keep; the build keep reads it from IMFS.
  printf 'args = ["keygen", "%s", "%s"]\n' "$KEY" "$W/func.pub.json" >"$W/keygen.toml"
  ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/keygen.toml" "$ENARX" run "$CLI" >"$W/keygen.log" 2>&1
  FKEY=$(sed -n 's/.*"keyid": "\([0-9a-f]\{8\}\).*/\1/p' "$W/func.pub.json" | head -1)
  files=("$CLI" "$KEY" "${files[@]}")
  CMD=("$CLI" run --name build --key "$KEY" --out "$W" --lstrip "$W/"
       --materials "$SRC"/*.c "$SRC"/*.h --products "$SRC/lua" "$SRC/luac" -- "${MAKE_CMD[@]}")
  DUMPS="$DUMPS;$W/build.$FKEY.link=$W/build.$FKEY.link"
else
  CMD=("${MAKE_CMD[@]}")
fi

PRE=$(printf '%s:' "${files[@]}")
{
  printf 'args = ['
  sep=""
  for a in "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nPRELOADS = "%s"\nDUMPS = "%s"\n' "${PRE%:}" "$DUMPS"
} >"$W/build.toml"

echo "preloads: ${#files[@]} files" >&2
start=$(date +%s.%N)
# One grate worker, as in lind-wasm-apps' in-toto/examples/gcc-imfs-sgx.sh.
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/build.toml" \
  timeout 3600 "$ENARX" run "$IMFS_GRATE" >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"

grep -E "error|Error|undefined" "$W/build.log" | grep -vE "^\[|failed to enable wasm cache" | head -5
for t in lua luac; do
  [ -x "$SRC/$t" ] && echo "$t: $(file -b "$SRC/$t" | cut -c1-60)" || echo "$t: missing"
done
[ -x "$SRC/lua" ] && "$SRC/lua" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))'
ls "$W"/build.*.link 2>/dev/null
exit $rc
