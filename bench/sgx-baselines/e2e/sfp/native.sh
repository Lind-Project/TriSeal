#!/bin/bash
# Build one single-file program natively with make and the native gcc
# 15.2.0 (the compiler Gramine runs), same flags as in the TEEs.
# Usage: e2e/sfp/native.sh <prog> [workdir]   (prints "wall <s>")
#   TWICE=1  build two copies one after the other (twobuild), "@build" lines.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
NG=$E/native-gcc/bin
p=$1
W=${2:-$E/work-sfp-native-$p}
rm -rf "$W" && mkdir -p "$W"
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$SFP/tree/$p" "$W/a"; cp -r "$SFP/tree/$p" "$W/b"; DIRS=("$W/a" "$W/b")
else
  cp -r "$SFP/tree/$p/." "$W/"; DIRS=("$W")
fi
CC="$NG/gcc -B$NG/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
ARGS=("CC=$CC" "CFLAGS=$SFP_CFLAGS" "LIBS=$(sfp_libs "$p")")
start=$(date +%s.%N)
if [ "${TWICE:-0}" = 1 ]; then
  "$NG/twobuild" "$NG/make" "${DIRS[@]}" -- "${ARGS[@]}" >"$W.log" 2>&1
else
  "$NG/make" -C "$W" "${ARGS[@]}" >"$W.log" 2>&1
fi
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined" "$W.log" | head -5
grep "^@build" "$W.log"
for d in "${DIRS[@]}"; do "$HERE/check.sh" "$p" "$d/$(sfp_out "$p")"; done
exit $rc
