#!/bin/bash
# Build Lua 5.5.1 with its own Makefile under Gramine-SGX, with the same
# compiler (gcc 15.2.0, a native build of the same source TriSeal's runs),
# flags, headers and libraries as e2e/lua-triseal.sh. make, gcc, cc1, as, ld,
# ar and ranlib each run in their own enclave; the toolchain and headers are
# trusted files. Gramine's tmpfs is private to a process, so the sources,
# objects and gcc's temporary files live on a host mount shared by all
# processes:
#   MODE=encrypted  Gramine encrypted files (sources encrypted up front)
#   MODE=allowed    plain host files (no protection)
#
# Usage: e2e/lua-gramine.sh [workdir]   (prints "wall <s>")
# Knobs: ENCLAVE_SIZE (default 1G), EDMM (default false).
#   TWICE=1  build two copies of the tree one after the other in the same
#            gramine-sgx instance (src/twobuild.c as the entrypoint); prints
#            "@build <n> <dir> <s> <rc>" for each.
#   PROBE=1  run `make --version` instead, to time startup on its own.
#   STATS=1  set sgx.enable_stats (needs a debug Gramine first on PATH).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
NG=$E/native-gcc/bin
MODE=${MODE:-encrypted}
W=${1:-$E/work-lua-gramine-$MODE}
KEYHEX=00112233445566778899aabbccddeeff

rm -rf "$W" "$W.manifest"*
mkdir -p "$W"
PLAIN=$E/lua-plain-$$
if [ "${TWICE:-0}" = 1 ]; then
  mkdir -p "$PLAIN"
  cp -r "$E/lua-5.5.1-src" "$PLAIN/a"
  cp -r "$E/lua-5.5.1-src" "$PLAIN/b"
  SRCS=("$W/a/src" "$W/b/src")
else
  cp -r "$E/lua-5.5.1-src" "$PLAIN"
  SRCS=("$W/src")
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

# Host programs and the shared libraries they load, plus the staged inputs.
# make is native-gcc/bin/make, GNU make 4.4 (the source lind's make is built
# from) configured with --disable-posix-spawn: glibc's posix_spawn uses
# clone(CLONE_VM|CLONE_VFORK), which Gramine rejects, so the distro make
# cannot start any child (error 127).
bins="/usr/bin/as /usr/bin/ld /usr/bin/ar /usr/bin/ranlib"
real=$(for b in $bins; do readlink -f "$b"; done)
# /lib is Gramine's runtime dir inside the enclave, so libraries are reached
# through /usr/lib/x86_64-linux-gnu. Trust both the soname symlink the loader
# opens and the file it points to.
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
  "$HERE/lua-gramine.manifest.template" "$M"
gramine-sgx-sign --manifest "$M" --output "$M.sgx" >/dev/null || exit 1

CC="$NG/gcc -std=gnu99 -B$NG/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
MAKE_ARGS=(all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=/usr/bin/ar rcu" RANLIB=/usr/bin/ranlib)
if [ "${PROBE:-0}" = 1 ]; then
  ARGS=(--version)
elif [ "${TWICE:-0}" = 1 ]; then
  ARGS=("$NG/make" "${SRCS[@]}" -- "${MAKE_ARGS[@]}")
else
  ARGS=(-C "$W/src" "${MAKE_ARGS[@]}")
fi
start=$(date +%s.%N)
(cd "$(dirname "$W")" && timeout 3600 ${TRACE:-} gramine-sgx "$(basename "$W")" "${ARGS[@]}") >"$W.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"
grep -E "error|Error|undefined" "$W.log" | head -5
grep "^@build" "$W.log"
[ "${PROBE:-0}" = 1 ] && exit $rc

# Check the result natively (decrypting it first in encrypted mode).
for d in "${SRCS[@]}"; do
  out=$d/lua
  if [ "$MODE" = encrypted ] && [ -f "$out" ]; then
    gramine-sgx-pf-crypt decrypt -w "$E/wrap.key" -i "$out" -o "$out.plain" >/dev/null && out=$out.plain && chmod +x "$out"
  fi
  [ -f "$out" ] && "$out" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))' || echo "lua: missing"
done
exit $rc
