#!/bin/bash
# Build Lua 5.5.1 natively, for reference: the same gcc, flags, headers and
# libraries as e2e/lua-gramine.sh, outside any TEE.
# Usage: e2e/lua-native.sh [workdir]   (prints "wall <s>")
#   TWICE=1  build two copies of the tree one after the other through
#            src/twobuild.c; prints "@build <n> <dir> <s> <rc>" for each.
set -uo pipefail
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
NG=$E/native-gcc/bin
W=${1:-$E/work-lua-native}

rm -rf "$W"
mkdir -p "$W"
CC="$NG/gcc -std=gnu99 -B$NG/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
MAKE_ARGS=(all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=/usr/bin/ar rcu" RANLIB=/usr/bin/ranlib)
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$E/lua-5.5.1-src" "$W/a"
  cp -r "$E/lua-5.5.1-src" "$W/b"
  SRCS=("$W/a/src" "$W/b/src")
  CMD=("$NG/twobuild" "$NG/make" "${SRCS[@]}" -- "${MAKE_ARGS[@]}")
else
  cp -r "$E/lua-5.5.1-src/." "$W/"
  SRCS=("$W/src")
  CMD=("$NG/make" -C "$W/src" "${MAKE_ARGS[@]}")
fi
start=$(date +%s.%N)
"${CMD[@]}" >"$W.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined" "$W.log" | head -5
grep "^@build" "$W.log"
for d in "${SRCS[@]}"; do
  [ -f "$d/lua" ] && "$d/lua" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))' || echo "lua: missing"
done
exit $rc
