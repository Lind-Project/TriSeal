#!/bin/bash
# The canary project (canary/proj) built in TriSeal's full setup, as
# e2e/lua-triseal-shm.sh builds Lua: the pipeline (gcc, binutils, make, the
# probe tool) and the sources in one bundle in shared memory, attested in
# the keep, make run by the IMFS grate under the in-toto grate, the outputs
# signed and dumped, the evidence checked up to the quote. Afterwards the
# dumped canary program is run on the host and prints what it recorded.
# Usage: canary/triseal.sh <workdir>     (prints "wall <s> rc=<rc>")
#   TARGET=time|env   make's goal (default time): tcanary + libcanary.a, or
#                     ecanary (canary/proj/Makefile)
#   CLOCK=<cwasm>     the logical-clock grate above make; give its epoch as
#                     GUEST_ENV='LOGICAL_CLOCK_EPOCH = "<secs>"' and an
#                     IMFS_GRATE that starts file times at that epoch
#   GUEST_ENV=<toml>  more [env], declared in the invocation (run-start)
#   ENARX, IMFS_GRATE, INTOTO_GRATE, ROOT   as in lua-triseal-shm.sh
#   SRCDIR=<dir>      the project tree to pack (default canary/proj); the
#                     host may lay it out as it likes (reprotest's file
#                     ordering); the paths in the keep are <workdir>/src
#   KEEP_BUNDLE=1     keep the bundle as <workdir>/bundle.prld
#   ENV_CLASSES=<c..> the classes `probe env` reads (default all; proj/probe.c)
set -uo pipefail
K=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
B=$HOME/sgx-bench
R=${ROOT:-$E/root-ebc}
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
ENARX=${ENARX:-$K/enarx-default}
INTOTO_GRATE=${INTOTO_GRATE:-$B/triseal/ebc/intoto-grate.cwasm}
IMFS_GRATE=${IMFS_GRATE:-$E/clang/ebc/imfs-grate-r.cwasm}
MKPRELOAD=${MKPRELOAD:-$B/grates-attest/tools/mkpreload/target/release/mkpreload}
PROBE=$K/ebc/probe
TARGET=${TARGET:-time}
W=${1:?usage: canary/triseal.sh <workdir>}
SRCDIR=${SRCDIR:-$K/proj}

rm -rf "$W"
mkdir -p "$W/src"
# The project without the tool's own source (the tool is part of the
# pipeline, as a code generator shipped with a toolchain would be).
(cd "$SRCDIR" && tar cf - --exclude=probe.c .) | (cd "$W/src" && tar xf -)
SRC=$W/src
CC="$BIN/gcc -B$BIN/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"

headers() {
  # -MG: stamp.h and env.h are written during the build
  gcc -M -MG -nostdinc -O2 -isystem "$G/include" -isystem "$R/usr/include" \
    "$K/proj/tcanary.c" "$K/proj/ecanary.c" "$K/proj/extra.c" |
    tr -d '\\' | tr ' ' '\n' | grep '^/' | grep -v "^$K/" | sort -u
}
pipeline=(
  "$BIN"/{make,gcc,cc1,as,ld,ar,ranlib} "$PROBE"
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a,libgcc_s.so,libgcc_s.so.1}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
)
[ -n "${CLOCK:-}" ] && pipeline=("$CLOCK" "${pipeline[@]}")
existing=()
for f in "${pipeline[@]}"; do [ -e "$f" ] && existing+=("$f"); done
# The sources in the order the host lists them (find, unsorted), so that a
# host reordering its directory reaches the bundle if anything does.
mapfile -t sources < <(find "$SRC" -type f)

BUNDLE=/dev/shm/canary.$$.prld
DUMP=/dev/shm/canary.$$.dump
"$MKPRELOAD" "$BUNDLE" "${existing[@]}" --section "${sources[@]}" 2>"$W/mkpreload.log" || { tail -3 "$W/mkpreload.log"; exit 1; }
grep "^mkpreload:" "$W/mkpreload.log" >&2

MAKE_ARGS=("$TARGET" "CC=$CC" "AR=$BIN/ar" "RANLIB=$BIN/ranlib -U" "PROBE=$PROBE" "ENV_CLASSES=${ENV_CLASSES:-}")
CMD=("$BIN/make" -C "$SRC" "${MAKE_ARGS[@]}")
[ -n "${CLOCK:-}" ] && CMD=("$CLOCK" "${CMD[@]}")
if [ "$TARGET" = time ]; then
  OUTS=(tcanary libcanary.a stamp.h early.h)
else
  OUTS=(ecanary env.h)
fi
DUMPS=""
for o in "${OUTS[@]}"; do DUMPS+="$SRC/$o=out/$o;"; done
DUMPS=${DUMPS%;}
{
  printf 'args = ['
  sep=""
  for a in "$IMFS_GRATE" --preload-shm --attest --dump-shm "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nDUMPS = "%s"\n' "$DUMPS"
  [ -n "${GUEST_ENV:-}" ] && printf '%s\n' "$GUEST_ENV"
  printf '\n[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((64 << 20))
} >"$W/build.toml"

start=$(date +%s.%N)
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/build.toml" \
  timeout 1800 "$ENARX" run "$INTOTO_GRATE" >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined|imfs-grate:|refused" "$W/build.log" | grep -vE "^\[fork|failed to enable wasm cache" | head -5
grep "^\[intoto-grate\] attested 3" "$W/build.log"
if [ -s "$DUMP" ]; then
  cp "$DUMP" "$W/dump.prld"
  "$MKPRELOAD" unpack "$W/dump.prld" "$W/dump" 2>&1 | tail -1
  python3 "$E/verify-attest.py" "$W/dump" "$BUNDLE" | tail -1
  for p in tcanary ecanary; do
    [ -x "$W/dump/out/$p" ] && "$W/dump/out/$p" > "$W/canary.txt"
  done
  (cd "$W/dump/out" && sha256sum "${OUTS[@]}") > "$W/products.sha256"
fi
[ "${KEEP_BUNDLE:-0}" = 1 ] && cp "$BUNDLE" "$W/bundle.prld"
rm -f "$BUNDLE" "$DUMP"
exit $rc
