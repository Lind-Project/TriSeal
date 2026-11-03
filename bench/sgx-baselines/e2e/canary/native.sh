#!/bin/bash
# The canary project (canary/proj) built natively with the native build of
# the evaluation's gcc (e2e/native-gcc, the staged sysroot of e2e/root), the
# host's binutils (as e2e/lua-native.sh) and the native build of probe. The
# caller's environment, umask, CPU set and so on reach the build: that is
# what the variations vary. Afterwards the canary program prints what it
# recorded.
# Usage: canary/native.sh <workdir>     (prints "wall <s> rc=<rc>")
#   TARGET=time|env  make's goal (default time);  SRCDIR=<dir> the tree to
#   copy (default canary/proj); ENV_CLASSES=<c..> the classes `probe env`
#   reads (default all)
set -uo pipefail
K=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
NG=$E/native-gcc/bin
TARGET=${TARGET:-time}
W=${1:?usage: canary/native.sh <workdir>}
SRCDIR=${SRCDIR:-$K/proj}
rm -rf "$W"
mkdir -p "$W/src"
(cd "$SRCDIR" && tar cf - --exclude=probe.c .) | (cd "$W/src" && tar xf -)
CC="$NG/gcc -B$NG/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
start=$(date +%s.%N)
(cd "$W/src" && "$NG/make" "$TARGET" "CC=$CC" AR=/usr/bin/ar "RANLIB=/usr/bin/ranlib -U" \
  "PROBE=$K/native-bin/probe" "ENV_CLASSES=${ENV_CLASSES:-}") >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
if [ "$TARGET" = time ]; then OUTS=(tcanary libcanary.a stamp.h early.h); else OUTS=(ecanary env.h); fi
mkdir -p "$W/dump/out"
for o in "${OUTS[@]}"; do cp -p "$W/src/$o" "$W/dump/out/$o" 2>/dev/null; done
for p in tcanary ecanary; do
  [ -x "$W/dump/out/$p" ] && "$W/dump/out/$p" > "$W/canary.txt"
done
(cd "$W/dump/out" && sha256sum "${OUTS[@]}") > "$W/products.sha256"
exit $rc
