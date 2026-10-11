#!/bin/bash
# The Lua build with its preloads in shared memory and attested inside the
# keep ("preloading signing"):
#
#   host   mkpreload packs the compiler pipeline and the Lua sources into one
#          bundle of two archives in /dev/shm; the keep maps it, and the file
#          the dump goes to, before lind starts ([shm] in Enarx.toml)
#   keep   the in-toto grate runs the IMFS grate, which
#          1. copies the compiler pipeline out of the bundle and stages it,
#          2. stops and has it attested: the in-toto grate reads the archive
#             out of IMFS memory, hashes every file and signs a statement,
#          3. resumes with the sources, and stops again to have them attested,
#          4. runs make; at exit lua, luac and the attestations go into one
#             dump archive, copied to shared memory and committed with one
#             ocall
#   host   the dump is unpacked into the work directory, lua is run, and the
#          attestations are checked against the bundle (verify-attest.py)
#
# Needs the staged tree from stage.sh under $E, the grates from
# lind-wasm-example-grates (branch imfs-preload-attest), mkpreload, and a
# TriSeal runtime with [shm] support (branch triseal-shm-preload).
# Usage: e2e/lua-triseal-shm.sh [workdir]     (prints "wall <s>" for the keep)
#   PROBE=1     run `make --version` instead of the build
#   SMALL=1     pack only make, as a one-archive bundle (use with PROBE=1)
#   NOATTEST=1  leave out --attest;  NODUMP=1  leave out --dump-shm
#   TAMPER=1    flip the last byte of the bundle after packing (a byte of the
#               last source file): the keep must refuse to run
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
ENARX=${ENARX:-$E/shm/TriSeal/target/release/enarx}
INTOTO_GRATE=${INTOTO_GRATE:-$HERE/triseal/grates/attest/intoto-grate.cwasm}
# The IMFS grate is exec'd by the in-toto grate. A TriSeal keep sees host
# paths (lind-boot outside a keep would chroot into lindfs instead).
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/attest/imfs-grate.cwasm}
MKPRELOAD=${MKPRELOAD:-$HOME/sgx-bench/grates-attest/tools/mkpreload/target/release/mkpreload}
W=${1:-$E/work-lua-shm}

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

pipeline=(
  "$BIN"/{make,gcc,cc1,as,ld,ar,ranlib}
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
)
sources=("$SRC"/*.c "$SRC"/*.h "$SRC"/Makefile)
sections=("${pipeline[@]}" --section "${sources[@]}")
if [ "${SMALL:-0}" = 1 ]; then
  sections=("$BIN/make")
fi

BUNDLE=/dev/shm/lua-shm.$$.prld
DUMP=/dev/shm/lua-shm.$$.dump
"$MKPRELOAD" "$BUNDLE" "${sections[@]}" 2>"$W/mkpreload.log" || { tail -3 "$W/mkpreload.log"; exit 1; }
grep "^mkpreload:" "$W/mkpreload.log" >&2
if [ "${TAMPER:-0}" = 1 ]; then
  size=$(stat -c %s "$BUNDLE")
  byte=$(od -An -tu1 -j $((size - 1)) -N1 "$BUNDLE" | tr -d ' ')
  printf "$(printf '\\%03o' $(((byte + 1) % 256)))" | dd of="$BUNDLE" bs=1 seek=$((size - 1)) conv=notrunc 2>/dev/null
  echo "tampered: last byte $byte -> $(((byte + 1) % 256))" >&2
fi

MAKE_ARGS=(all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=$BIN/ar rcu" "RANLIB=$BIN/ranlib")
CMD=("$BIN/make" -C "$SRC" "${MAKE_ARGS[@]}")
[ "${PROBE:-0}" = 1 ] && CMD=("$BIN/make" --version)
FLAGS=(--preload-shm)
[ "${NOATTEST:-0}" = 1 ] || FLAGS+=(--attest)
[ "${NODUMP:-0}" = 1 ] || FLAGS+=(--dump-shm)
DUMPS="$SRC/lua=out/lua;$SRC/luac=out/luac"
[ "${PROBE:-0}" = 1 ] && DUMPS=""

{
  printf 'args = ['
  sep=""
  for a in "$IMFS_GRATE" "${FLAGS[@]}" "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nDUMPS = "%s"\n\n' "$DUMPS"
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((256 << 20))
} >"$W/build.toml"

start=$(date +%s.%N)
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/build.toml" \
  timeout 3600 ${TRACE:-} "$ENARX" run "$INTOTO_GRATE" >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"

grep -E "error|Error|undefined|imfs-grate:" "$W/build.log" | grep -vE "^\[fork|failed to enable wasm cache" | head -5
grep "^\[intoto-grate\]" "$W/build.log"
if [ -s "$DUMP" ]; then
  cp "$DUMP" "$W/dump.prld"
  "$MKPRELOAD" unpack "$W/dump.prld" "$W/dump" 2>&1 | tail -1
  if [ -x "$W/dump/out/lua" ]; then
    echo "lua: $(file -b "$W/dump/out/lua" | cut -c1-60)"
    "$W/dump/out/lua" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))'
  fi
  [ -d "$W/dump/attest" ] && python3 "$HERE/e2e/verify-attest.py" "$W/dump/attest" "$BUNDLE"
fi
rm -f "$BUNDLE" "$DUMP"
exit $rc
