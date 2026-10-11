#!/bin/bash
# Build one single-file program under Gramine-SGX, like e2e/lua-gramine.sh:
# make, gcc, cc1, as and ld each in their own enclave, the toolchain trusted,
# the source and gcc's temporaries on a host mount shared by the processes:
#   MODE=encrypted  Gramine encrypted files (the source encrypted up front)
#   MODE=allowed    plain host files
# Usage: e2e/sfp/gramine.sh <prog> [workdir]   (prints "wall <s>")
# Knobs as in lua-gramine.sh: ENCLAVE_SIZE (default 1G), EDMM, PROBE=1,
# TWICE=1, STATS=1.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
NG=$E/native-gcc/bin
MODE=${MODE:-encrypted}
p=$1
W=${2:-$E/work-sfp-gramine-$MODE-$p}
KEYHEX=00112233445566778899aabbccddeeff

rm -rf "$W" "$W.manifest"*
mkdir -p "$W"
PLAIN=$E/sfp-plain-$$
mkdir -p "$PLAIN"
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$SFP/tree/$p" "$PLAIN/a"; cp -r "$SFP/tree/$p" "$PLAIN/b"; DIRS=("$W/a" "$W/b")
else
  cp -r "$SFP/tree/$p/." "$PLAIN/"; DIRS=("$W")
fi
mkdir -p "$PLAIN/tmp"
if [ "$MODE" = encrypted ]; then
  printf '%s' "$KEYHEX" | xxd -r -p >"$E/wrap.key"
  gramine-sgx-pf-crypt encrypt -w "$E/wrap.key" -i "$PLAIN" -o "$W" >/dev/null 2>&1
  mkdir -p "$W/tmp"
else
  cp -r "$PLAIN/." "$W/"
fi
rm -rf "$PLAIN"

bins="/usr/bin/as /usr/bin/ld"
real=$(for b in $bins; do readlink -f "$b"; done)
libs=$(for b in $bins $NG/make $NG/twobuild $NG/gcc $NG/cc1; do ldd "$(readlink -f "$b")" | awk '/=> \//{print $3}'; done |
  sed 's#^/lib/#/usr/lib/#' | sort -u)
libs="$libs $(printf '%s\n' $libs | xargs -r readlink -f | sort -u)"
trusted=$(printf '%s\n' $bins $real $libs "$R/usr/include/" "$G/" "$L/" "$R/lib/x86_64-linux-gnu/" "$R/lib64/" | sort -u | paste -sd,)

M=$W.manifest
ENTRY=$NG/make
[ "${TWICE:-0}" = 1 ] && ENTRY=$NG/twobuild
STATS_FLAG=false
[ "${STATS:-0}" = 1 ] && STATS_FLAG=true
gramine-manifest -Dmake="$ENTRY" -Dstats="$STATS_FLAG" -Dwork="$W" -Droot="$R" -Dgcc="$NG" -Dmode="$MODE" \
  -Denclave_size="${ENCLAVE_SIZE:-1G}" -Dedmm="${EDMM:-false}" -Dtrusted="$trusted" \
  "$HERE/../lua-gramine.manifest.template" "$M"
gramine-sgx-sign --manifest "$M" --output "$M.sgx" >/dev/null || exit 1

CC="$NG/gcc -B$NG/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
MARGS=("CC=$CC" "CFLAGS=$SFP_CFLAGS" "LIBS=$(sfp_libs "$p")")
if [ "${PROBE:-0}" = 1 ]; then
  ARGS=(--version)
elif [ "${TWICE:-0}" = 1 ]; then
  ARGS=("$NG/make" "${DIRS[@]}" -- "${MARGS[@]}")
else
  ARGS=(-C "$W" "${MARGS[@]}")
fi
start=$(date +%s.%N)
(cd "$(dirname "$W")" && timeout 7200 ${TRACE:-} gramine-sgx "$(basename "$W")" "${ARGS[@]}") >"$W.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined" "$W.log" | head -5
grep "^@build" "$W.log"
[ "${PROBE:-0}" = 1 ] && exit $rc
for d in "${DIRS[@]}"; do
  out=$d/$(sfp_out "$p")
  if [ "$MODE" = encrypted ] && [ -f "$out" ]; then
    gramine-sgx-pf-crypt decrypt -w "$E/wrap.key" -i "$out" -o "$out.plain" >/dev/null && out=$out.plain && chmod +x "$out"
  fi
  "$HERE/check.sh" "$p" "$out"
done
exit $rc
