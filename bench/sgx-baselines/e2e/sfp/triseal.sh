#!/bin/bash
# Build one single-file program inside a TriSeal SGX keep, like
# e2e/lua-triseal.sh: make, gcc, cc1, as and ld run as cages under the IMFS
# grate, with the toolchain, libraries and the source preloaded into IMFS;
# the binary is dumped to the host at exit and smoke-tested natively.
# Usage: e2e/sfp/triseal.sh <prog> [workdir]   (prints "wall <s>")
#   PROBE=1  run `make --version` with the same preloads (startup alone)
#   BACKEND=nil  run the keep as a plain process, without SGX
#   LIND_BOOT=<lind-boot>  run the grate and cages under lind-boot, no TEE
#   TWICE=1  build two copies one after the other in the same keep
#            (twobuild), "@build <n> <dir> <s> <rc>" for each
#   NODUMP=1 keep the binary in the keep
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
BENCH=$(cd "$HERE/../.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
ENARX=${ENARX:-$E/ze/TriSeal/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$BENCH/triseal/grates/ze/imfs-grate.cwasm}
TWOBUILD=$BENCH/triseal/build/twobuild.cwasm
p=$1
W=${2:-$E/work-sfp-triseal-$p}
rm -rf "$W" && mkdir -p "$W"
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$SFP/tree/$p" "$W/a"; cp -r "$SFP/tree/$p" "$W/b"; DIRS=("$W/a" "$W/b")
else
  cp -r "$SFP/tree/$p/." "$W/"; DIRS=("$W")
fi
CC="$BIN/gcc -B$BIN/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
ARGS=("CC=$CC" "CFLAGS=$SFP_CFLAGS" "LIBS=$(sfp_libs "$p")")

# Headers the program reads, from the host compiler with the same search path.
headers() {
  (cd "${DIRS[0]}" && gcc -M -nostdinc $SFP_CFLAGS -isystem "$G/include" -isystem "$R/usr/include" "$p.c") |
    tr -d '\\' | tr ' ' '\n' | grep '^/' | grep -v "^$W/" | sort -u
}

files=(
  "$BIN"/{make,gcc,cc1,as,ld}
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
)
DUMPS=""
for d in "${DIRS[@]}"; do
  files+=("$d/$p.c" "$d/Makefile" "$d/compat/sys/sysctl.h")
  DUMPS+="$d/$(sfp_out "$p")=$d/$(sfp_out "$p");"
done
DUMPS=${DUMPS%;}
[ "${NODUMP:-0}" = 1 ] && DUMPS=""
CMD=("$BIN/make" -C "${DIRS[0]}" "${ARGS[@]}")
if [ "${TWICE:-0}" = 1 ]; then
  files=("$TWOBUILD" "${files[@]}")
  CMD=("$TWOBUILD" "$BIN/make" "${DIRS[@]}" -- "${ARGS[@]}")
fi
[ "${PROBE:-0}" = 1 ] && CMD=("$BIN/make" --version)

PRE=$(printf '%s:' "${files[@]}")
{
  printf 'args = ['
  sep=""
  for a in "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nPRELOADS = "%s"\nDUMPS = "%s"\n' "${PRE%:}" "$DUMPS"
} >"$W/build.toml"

start=$(date +%s.%N)
if [ -n "${LIND_BOOT:-}" ]; then
  # The same grate and cages under lind-wasm's own runtime, outside any TEE
  # (a lind-boot that skips its chroot when LIND_NO_CHROOT is set).
  LIND_NO_CHROOT=1 LIND_GRATE_WORKERS=1 timeout 7200 ${TRACE:-} "$LIND_BOOT" \
    --env "PRELOADS=${PRE%:}" --env "DUMPS=$DUMPS" "$IMFS_GRATE" "${CMD[@]}" >"$W/build.log" 2>&1
else
  LIND_GRATE_WORKERS=1 ENARX_BACKEND=${BACKEND:-sgx} ENARX_WASMCFGFILE="$W/build.toml" \
    timeout 7200 ${TRACE:-} "$ENARX" run "$IMFS_GRATE" >"$W/build.log" 2>&1
fi
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined" "$W/build.log" | grep -vE "^\[|failed to enable wasm cache" | head -5
grep "^@build" "$W/build.log"
[ "${PROBE:-0}" = 1 ] || [ "${NODUMP:-0}" = 1 ] && exit $rc
for d in "${DIRS[@]}"; do "$HERE/check.sh" "$p" "$d/$(sfp_out "$p")"; done
exit $rc
