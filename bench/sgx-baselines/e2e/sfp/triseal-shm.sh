#!/bin/bash
# Build one single-file program in a TriSeal keep with its preloads in shared
# memory, attested inside the keep, as e2e/lua-triseal-shm.sh does for Lua:
# mkpreload packs the compiler pipeline and the program's sources as two
# archives of one bundle in /dev/shm; the in-toto grate runs the IMFS grate,
# which copies the bundle in, has each archive attested, runs make, and puts
# the output and the attestations in one dump archive the host cuts at exit.
# Usage: e2e/sfp/triseal-shm.sh <prog> [workdir]   (prints "wall <s>")
#   ROOT=<dir>  the staged tree (toolchain and sysroot) to build with
#   PROBE=1  run `make --version` with the same preloads (startup alone)
#   TWICE=1  build two copies one after the other in the same keep
#            (twobuild), "@build <n> <dir> <s> <rc>" for each; both outputs
#            are signed and dumped
#   SMALL=1  pack only make, as a one-archive bundle (use with PROBE=1)
#   NOATTEST=1  leave out --attest
#   NOINTOTO=1  run the IMFS grate as the main module, without the in-toto
#               grate (and so without attestation)
#   BACKEND=nil  run the keep as a plain process, without SGX
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
BENCH=$(cd "$HERE/../.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=${ROOT:-$E/root}
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
ENARX=${ENARX:-$E/ze/TriSeal/target/release/enarx}
INTOTO_GRATE=${INTOTO_GRATE:-$BENCH/triseal/grates/ze/intoto-grate.cwasm}
IMFS_GRATE=${IMFS_GRATE:-$BENCH/triseal/grates/ze/imfs-grate.cwasm}
MKPRELOAD=${MKPRELOAD:-$HOME/sgx-bench/grates-attest/tools/mkpreload/target/release/mkpreload}
TWOBUILD=$BENCH/triseal/build/twobuild.cwasm
p=$1
W=${2:-$E/work-sfp-triseal-shm-$p}
rm -rf "$W" && mkdir -p "$W"
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$SFP/tree/$p" "$W/a"; cp -r "$SFP/tree/$p" "$W/b"; DIRS=("$W/a" "$W/b")
else
  cp -r "$SFP/tree/$p/." "$W/"; DIRS=("$W")
fi
CC="$BIN/gcc -B$BIN/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
OUTF=$(sfp_out "$p")

headers() {
  (cd "${DIRS[0]}" && gcc -M -nostdinc $SFP_CFLAGS -isystem "$G/include" -isystem "$R/usr/include" "$p.c") |
    tr -d '\\' | tr ' ' '\n' | grep '^/' | grep -v "^$W/" | sort -u
}
pipeline=(
  "$BIN"/{make,gcc,cc1,as,ld}
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
)
[ "${TWICE:-0}" = 1 ] && pipeline=("$TWOBUILD" "${pipeline[@]}")
sources=()
for d in "${DIRS[@]}"; do sources+=("$d/$p.c" "$d/Makefile" "$d/compat/sys/sysctl.h"); done

BUNDLE=/dev/shm/sfp-shm.$$.prld
DUMP=/dev/shm/sfp-shm.$$.dump
sections=("${pipeline[@]}" --section "${sources[@]}")
[ "${SMALL:-0}" = 1 ] && sections=("$BIN/make")
"$MKPRELOAD" "$BUNDLE" "${sections[@]}" 2>"$W/mkpreload.log" ||
  { tail -3 "$W/mkpreload.log"; exit 1; }
grep "^mkpreload:" "$W/mkpreload.log" >&2

ARGS=("CC=$CC" "CFLAGS=$SFP_CFLAGS" "LIBS=$(sfp_libs "$p")")
CMD=("$BIN/make" -C "$W" "${ARGS[@]}")
DUMPS="$W/$OUTF=out/$OUTF"
if [ "${TWICE:-0}" = 1 ]; then
  CMD=("$TWOBUILD" "$BIN/make" "${DIRS[@]}" -- "${ARGS[@]}")
  DUMPS="$W/a/$OUTF=out/a/$OUTF;$W/b/$OUTF=out/b/$OUTF"
fi
if [ "${PROBE:-0}" = 1 ]; then CMD=("$BIN/make" --version); DUMPS=""; fi
{
  printf 'args = ['
  sep=""
  FLAGS=(--preload-shm)
  [ "${NOATTEST:-0}" = 1 ] || [ "${NOINTOTO:-0}" = 1 ] || FLAGS+=(--attest)
  FLAGS+=(--dump-shm)
  MAIN=("$IMFS_GRATE")
  [ "${NOINTOTO:-0}" = 1 ] && MAIN=()
  for a in "${MAIN[@]}" "${FLAGS[@]}" "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nDUMPS = "%s"\n\n' "$DUMPS"
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((256 << 20))
} >"$W/build.toml"

start=$(date +%s.%N)
LIND_GRATE_WORKERS=1 ENARX_BACKEND=${BACKEND:-sgx} ENARX_WASMCFGFILE="$W/build.toml" \
  timeout 7200 ${TRACE:-} "$ENARX" run "$([ "${NOINTOTO:-0}" = 1 ] && echo "$IMFS_GRATE" || echo "$INTOTO_GRATE")" >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined|imfs-grate:" "$W/build.log" | grep -vE "^\[fork|failed to enable wasm cache" | head -5
grep "^\[intoto-grate\]" "$W/build.log"
grep "^@build" "$W/build.log"
if [ -s "$DUMP" ]; then
  cp "$DUMP" "$W/dump.prld"
  "$MKPRELOAD" unpack "$W/dump.prld" "$W/dump" 2>&1 | tail -1
  for o in "$W/dump/out/$OUTF" "$W"/dump/out/*/"$OUTF"; do
    [ -e "$o" ] && "$HERE/check.sh" "$p" "$o"
  done
  # The evidence (evidence/, or attest/ from an older recorder) against the
  # bundle and the dumped output.
  if [ -d "$W/dump/evidence" ] || [ -d "$W/dump/attest" ]; then
    python3 "$BENCH/e2e/verify-attest.py" "$W/dump" "$BUNDLE"
  fi
fi
rm -f "$BUNDLE" "$DUMP"
exit $rc
