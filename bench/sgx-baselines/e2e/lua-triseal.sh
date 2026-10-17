#!/bin/bash
# Build Lua 5.5.1 with its own Makefile inside a TriSeal SGX keep: make, gcc,
# cc1, as, ld, ar and ranlib all run as cages under the imfs grate, so every
# source, header, object and archive stays in enclave memory. lua and luac
# are dumped back to the host at exit and smoke-tested natively: IMFS packs
# them into one archive in the keep's dump region (--dump-shm, a host file
# mapped before lind starts), which is unpacked into the work directory. A
# sealed keep (TriSeal triseal-shim-seal) allows no other way out.
#
# Needs the staged tree from stage.sh under $E (default ~/sgx-bench/e2e).
# Usage: e2e/lua-triseal.sh [workdir]     (prints "wall <s>" for the keep)
#   INTOTO=1 wraps make in `in-toto-cli run`, signing a link over the
#   sources (materials) and lua/luac (products) with a key made in the keep.
#   PROBE=1 runs `make --version` with the same preloads instead, to measure
#   keep startup plus preloading on their own.
#   NODUMP=1 leaves lua and luac in the keep (no DUMPS), so that counting
#   enclave transitions as build minus probe covers only the compile.
#   FILEDUMP=1 has IMFS write each file back to the host itself at exit
#   (open/write through the runtime), as before --dump-shm; a sealed keep
#   refuses that.
#   TWICE=1 builds two copies of the tree one after the other in the same
#   keep (src/twobuild.c) and prints "@build <n> <dir> <s> <rc>" for each:
#   the first build is cold, the second runs with the module cache and
#   IMFS already warm.
#   IPC=1 stacks the IPC grate under IMFS (in-enclave pipes), see below.
#   GRATE_WORKERS=<n> sets lind's grate worker count, see below.
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
R=${ROOT:-$E/root}  # ROOT: the staged tree (toolchain and sysroot)
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
BIN=$R/usr/local/bin
MAKEPROG=${MAKEPROG:-$BIN/make}  # another make (cwasm), e.g. e2e/wasm/make-pselect.opt.cwasm
ENARX=${ENARX:-$HOME/sgx-bench/e2e/ts/TriSeal36/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/imfs-grate-clock.cwasm}
IPC_GRATE=${IPC_GRATE:-$HERE/triseal/grates/r3/ipc-grate.cwasm}
TWOBUILD=$HERE/triseal/build/twobuild.cwasm
CLI=${CLI:-$E/intoto/in-toto-cli.cwasm}
MKPRELOAD=${MKPRELOAD:-$HOME/sgx-bench/grates-attest/tools/mkpreload/target/release/mkpreload}
W=${1:-$E/work-lua}

rm -rf "$W"
mkdir -p "$W"
if [ "${TWICE:-0}" = 1 ]; then
  cp -r "$E/lua-5.5.1-src" "$W/a"
  cp -r "$E/lua-5.5.1-src" "$W/b"
  SRC=$W/a/src
  SRCS=("$W/a/src" "$W/b/src")
else
  cp -r "$E/lua-5.5.1-src/." "$W/"
  SRC=$W/src
  SRCS=("$SRC")
fi
CC="$BIN/gcc -std=gnu99 -B$BIN/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"

# Headers the build reads, from the host compiler with the same search path.
headers() {
  gcc -M -nostdinc -std=gnu99 -O2 -DLUA_USE_POSIX -isystem "$G/include" -isystem "$R/usr/include" "$SRC"/*.c |
    tr -d '\\' | tr ' ' '\n' | grep '^/' | grep -v "^$W/" | sort -u
}

files=(
  "$MAKEPROG" "$BIN"/{gcc,cc1,as,ld,ar,ranlib}
  "$L"/{crt1.o,crti.o,crtn.o,libc.so,libc_nonshared.a,libm.so}
  "$G"/{crtbegin.o,crtend.o,libgcc.a}
  "$R"/lib/x86_64-linux-gnu/{libc.so.6,libm.so.6,libmvec.so.1}
  "$R"/lib64/ld-linux-x86-64.so.2
  $(headers)
)
for d in "${SRCS[@]}"; do
  files+=("$d"/*.c "$d"/*.h "$d"/Makefile)
done
# Absolute tool paths: nothing is on PATH inside the keep.
MAKE_ARGS=(all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=$BIN/ar rcu" "RANLIB=$BIN/ranlib")
# JOBS=<n>: make -j<n>; JOBSTYLE=fifo|pipe: make's --jobserver-style.
[ -n "${JOBSTYLE:-}" ] && MAKE_ARGS=("--jobserver-style=$JOBSTYLE" "${MAKE_ARGS[@]}")
[ -n "${JOBS:-}" ] && MAKE_ARGS=("-j$JOBS" "${MAKE_ARGS[@]}")
# MAKE_EXTRA: more make options, space-separated (e.g. --debug=j).
[ -n "${MAKE_EXTRA:-}" ] && MAKE_ARGS=($MAKE_EXTRA "${MAKE_ARGS[@]}")
MAKE_CMD=("$MAKEPROG" -C "$SRC" "${MAKE_ARGS[@]}")
# DUMPS: IMFS path = where the file goes, on the host (FILEDUMP=1) or in
# the dump archive, relative to $W, which it is unpacked into.
DUMPS=""
for d in "${SRCS[@]}"; do
  if [ "${FILEDUMP:-0}" = 1 ]; then
    DUMPS+="$d/lua=$d/lua;$d/luac=$d/luac;"
  else
    DUMPS+="$d/lua=${d#$W/}/lua;$d/luac=${d#$W/}/luac;"
  fi
done
DUMPS=${DUMPS%;}
[ "${NODUMP:-0}" = 1 ] && DUMPS=""
if [ "${TWICE:-0}" = 1 ]; then
  files=("$TWOBUILD" "${files[@]}")
  MAKE_CMD=("$TWOBUILD" "$MAKEPROG" "${SRCS[@]}" -- "${MAKE_ARGS[@]}")
fi

if [ "${PROBE:-0}" = 1 ]; then
  MAKE_CMD=("$MAKEPROG" --version)
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
  if [ "${FILEDUMP:-0}" = 1 ]; then
    DUMPS="$DUMPS;$W/build.$FKEY.link=$W/build.$FKEY.link"
  else
    DUMPS="$DUMPS;$W/build.$FKEY.link=build.$FKEY.link"
  fi
else
  CMD=("${MAKE_CMD[@]}")
fi

# IPC=1: run the command under the IPC grate, itself under IMFS (grates
# chain: each execs the next), so that pipes, e.g. make's jobserver
# (JOBSTYLE=pipe), stay in enclave memory: IMFS alone has no pipe() (ENOSYS).
if [ "${IPC:-0}" = 1 ]; then
  files=("$IPC_GRATE" "${files[@]}")
  CMD=("$IPC_GRATE" "${CMD[@]}")
fi

# The dump region: a host file the keep maps before lind starts, which IMFS
# copies the DUMPS archive into at exit (IMFS options precede the program).
DUMP=""
if [ "${NODUMP:-0}" != 1 ] && [ "${FILEDUMP:-0}" != 1 ]; then
  DUMP=/dev/shm/lua-triseal.$$.dump
  CMD=(--dump-shm "${CMD[@]}")
fi

PRE=$(printf '%s:' "${files[@]}")
{
  printf 'args = ['
  sep=""
  for a in "${CMD[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nPRELOADS = "%s"\nDUMPS = "%s"\n' "${PRE%:}" "$DUMPS"
  [ -n "${GRATE_WORKERS:-}" ] && printf 'LIND_GRATE_WORKERS = "%s"\n' "$GRATE_WORKERS"
  [ -n "$DUMP" ] && printf '\n[shm]\ndump = "%s"\ndump_size = %d\n' "$DUMP" $((256 << 20))
} >"$W/build.toml"

echo "preloads: ${#files[@]} files" >&2
start=$(date +%s.%N)
# Grate workers: lind-3i reads LIND_GRATE_WORKERS from the process
# environment inside the keep, which the shim sets to LANG=C only, so a host
# variable never reaches it and every grate gets MAX_GRATE_WORKERS (3 in
# lind enarx-7-13). GRATE_WORKERS=<n> writes it into [env] instead, which
# takes effect with a TriSeal that copies LIND_* from [env] into its
# environment (e2e/r3w; at most MAX_GRATE_WORKERS, 8 there).
ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/build.toml" \
  timeout "${KEEP_TMO:-3600}" ${TRACE:-} "$ENARX" run "$IMFS_GRATE" >"$W/build.log" 2>&1
rc=$?
end=$(date +%s.%N)
echo "wall $(echo "$end - $start" | bc) rc=$rc"

grep -E "error|Error|undefined" "$W/build.log" | grep -vE "^\[|failed to enable wasm cache" | head -5
grep -E "^\[shim\]" "$W/build.log"
if [ -n "$DUMP" ]; then
  [ -s "$DUMP" ] && "$MKPRELOAD" unpack "$DUMP" "$W" 2>&1 | tail -1
  rm -f "$DUMP"
fi
grep "^@build" "$W/build.log"
for d in "${SRCS[@]}"; do
  for t in lua luac; do
    [ -x "$d/$t" ] && echo "$t: $(file -b "$d/$t" | cut -c1-60)" || echo "$t: missing"
  done
  [ -x "$d/lua" ] && "$d/lua" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))'
done
ls "$W"/build.*.link 2>/dev/null
exit $rc
