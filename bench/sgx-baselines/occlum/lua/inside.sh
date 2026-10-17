#!/bin/bash
# Run inside the Occlum container (e2e/lua-occlum.sh starts it): build Lua
# 5.5.1 with its own Makefile inside an Occlum enclave, with the same
# compiler (gcc 15.2.0), flags, headers and libraries as e2e/lua-gramine.sh
# and e2e/lua-triseal.sh.
#   inst/ is made once: the tools of occlum/sfp (/bin/{make,gcc,cc1}, the
#   container's as, ld, ar and ranlib, busybox for cp), twobuild, the staged
#   sysroot and three copies of the Lua tree (src/ for one build, a/ and b/
#   for TWICE), all in Occlum's encrypted root file system (SEFS); /tmp
#   (gcc's temporary files) is in enclave memory (ramfs) and /host is a host
#   directory the results are copied to.
# Usage: inside.sh   (prints "wall <s> rc=<rc>" for `occlum run`)
#   TWICE=1  build a/ and b/ one after the other in the same `occlum run`
#            (src/twobuild.c with vfork: Occlum has no fork); prints
#            "@build <n> <dir> <s> <rc>" for each.
#   PROBE=1  run `make --version` instead, to time startup on its own.
#   EDMM=1   build the instance for SGX2/EDMM (inst-edmm/): the enclave's
#            address range is the size of TriSeal's (2^36 = 64 GiB, shim-sgx
#            ENCL_SIZE_BITS) and memory past the initial sizes is committed
#            when first touched, as in TriSeal; otherwise SGX1 (inst/), with
#            every page of a 1 GB user space added at startup
#            (inst-<size>/ for another OCCLUM_USER_SPACE).
#   Sizes: OCCLUM_USER_SPACE, OCCLUM_KHEAP (initial), and with EDMM=1
#   OCCLUM_USER_SPACE_MAX, OCCLUM_KHEAP_MAX.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SFPT=$HERE/../sfp
E=$LUA_E
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
W=$E/work-lua-occlum
if [ "${EDMM:-0}" = 1 ]; then
  I=$HERE/inst-edmm
  # 55 GB user space (+ 1 GB the SDK adds to it) + 2 GB kernel heap +
  # 4096 TCSs with 1 MB stacks (TCSMaxNum is 4096 under EDMM whatever
  # max_num_of_threads says) + LibOS: about 62.5 GiB, which the SGX SDK
  # rounds up to a power of two, 64 GiB (60 GB came out at 128 GiB).
  US=${OCCLUM_USER_SPACE:-256MB} USMAX=${OCCLUM_USER_SPACE_MAX:-55GB}
  KH=${OCCLUM_KHEAP:-64MB} KHMAX=${OCCLUM_KHEAP_MAX:-2GB}
else
  US=${OCCLUM_USER_SPACE:-1GB} USMAX=$US
  I=$HERE/inst
  [ "$US" != 1GB ] && I=$HERE/inst-$US
  KH=${OCCLUM_KHEAP:-256MB} KHMAX=$KH
fi

if [ ! -f "$I/.built" ]; then
  rm -rf "$I" "$HERE/stage"
  [ -x "$SFPT/tools-strip/cc1" ] || { echo "no $SFPT/tools-strip: run occlum/sfp first"; exit 1; }
  gcc -O2 -Dfork=vfork -o "$HERE/twobuild" "$HERE/../../src/twobuild.c"
  mkdir -p "$HERE/stage"
  for d in src a b; do
    if [ "$d" = src ]; then cp -r "$E/lua-5.5.1-src/src" "$HERE/stage/src"
    else cp -r "$E/lua-5.5.1-src" "$HERE/stage/$d"; fi
  done
  cat >"$HERE/lua.yaml" <<YAML
includes:
  - base.yaml
targets:
  - target: /bin
    copy:
      - files:
        - $SFPT/tools-strip/gcc
        - $SFPT/tools-strip/cc1
        - $SFPT/tools-strip/make
        - $HERE/twobuild
        - /opt/occlum/toolchains/busybox/glibc/busybox
  - target: /usr/bin
    copy:
      - files:
        - /usr/bin/x86_64-linux-gnu-as
        - /usr/bin/x86_64-linux-gnu-ld.bfd
        - /usr/bin/x86_64-linux-gnu-ar
        - /usr/bin/x86_64-linux-gnu-ranlib
  - target: $R/usr
    copy:
      - dirs:
        - $R/usr/include
  - target: $R/usr/lib
    copy:
      - dirs:
        - $L
  - target: $R/usr/local/lib/gcc/x86_64-linux-gnu
    copy:
      - dirs:
        - $G
  - target: $R/lib
    copy:
      - dirs:
        - $R/lib/x86_64-linux-gnu
  - target: $R
    copy:
      - dirs:
        - $R/lib64
  - target: $W
    copy:
      - dirs:
        - $HERE/stage/src
        - $HERE/stage/a
        - $HERE/stage/b
YAML
  occlum new "$I" >/dev/null
  cd "$I"
  rm -rf image
  copy_bom -f "$HERE/lua.yaml" --root image --include-dir /opt/occlum/etc/template >copy_bom.log 2>&1
  # as and ld also in /bin, a -B prefix, so that gcc starts them by full
  # path (a PATH search, posix_spawnp, fails in Occlum); make starts ar and
  # ranlib by the full paths in AR and RANLIB.
  for t in as ar ranlib; do ln -sf "x86_64-linux-gnu-$t" "image/usr/bin/$t"; done
  ln -sf x86_64-linux-gnu-ld.bfd image/usr/bin/ld
  ln -sf ../usr/bin/x86_64-linux-gnu-as image/bin/as
  ln -sf ../usr/bin/x86_64-linux-gnu-ld.bfd image/bin/ld
  # Memory sizes as set above (SGX1: those of occlum/sfp for oggenc), /tmp
  # in enclave memory, /host for the result.
  jq '.resource_limits.user_space_size = "'"$US"'"
    | .resource_limits.user_space_max_size = "'"$USMAX"'"
    | .resource_limits.kernel_space_heap_size = "'"$KH"'"
    | .resource_limits.kernel_space_heap_max_size = "'"$KHMAX"'"
    | .resource_limits.init_num_of_threads = 16
    | .resource_limits.max_num_of_threads = 16
    | .feature.enable_edmm = '"$([ "${EDMM:-0}" = 1 ] && echo true || echo false)"'
    | .process.default_stack_size = "8MB"
    | .env.default = ((.env.default // []) + ["PATH=/bin:/usr/bin"])
    | .mount = ([.mount[] | select(.target != "/tmp" and .target != "/host")]
        + [{"target": "/tmp", "type": "ramfs"}, {"target": "/host", "type": "hostfs", "source": "./host"}])' \
    Occlum.json >o.json && mv o.json Occlum.json
  mkdir -p host
  occlum build $([ "${EDMM:-0}" = 1 ] && echo --enable-edmm Y) >build.log 2>&1
  touch .built
fi

cd "$I"
rm -f host/lua.*
# SEFS keeps what a run writes: remove the last builds' objects and
# programs first (not timed), or make finds them up to date.
if [ "${PROBE:-0}" != 1 ]; then
  occlum run /bin/twobuild /bin/make "$W/src" "$W/a/src" "$W/b/src" -- clean "RM=/bin/busybox rm -f" \
    >"$HERE/clean.log" 2>&1
fi
CC="/bin/gcc -std=gnu99 -B/bin/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
MAKE_ARGS=(all SYSCFLAGS=-DLUA_USE_POSIX "CC=$CC" "AR=/usr/bin/ar rcu" RANLIB=/usr/bin/ranlib)
# JOBS=<n>: make -j<n>; JOBSTYLE=fifo|pipe: make's --jobserver-style.
[ -n "${JOBSTYLE:-}" ] && MAKE_ARGS=("--jobserver-style=$JOBSTYLE" "${MAKE_ARGS[@]}")
[ -n "${JOBS:-}" ] && MAKE_ARGS=("-j$JOBS" "${MAKE_ARGS[@]}")
if [ "${PROBE:-0}" = 1 ]; then
  RUN=(occlum run /bin/make --version)
  SRCS=()
elif [ "${TWICE:-0}" = 1 ]; then
  SRCS=("$W/a/src" "$W/b/src")
  RUN=(occlum run /bin/twobuild /bin/make "${SRCS[@]}" -- "${MAKE_ARGS[@]}")
else
  SRCS=("$W/src")
  RUN=(occlum run /bin/make -C "$W/src" "${MAKE_ARGS[@]}")
fi
start=$(date +%s%N)
set +e
"${RUN[@]}" >"$HERE/run.log" 2>&1
rc=$?
set -e
end=$(date +%s%N)
ns=$((end - start))
printf 'wall %d.%09d rc=%d\n' $((ns / 1000000000)) $((ns % 1000000000)) "$rc"
grep -E "error|Error|undefined" "$HERE/run.log" | head -5 || true
grep "^@build" "$HERE/run.log" || true
if grep -q "Nothing to be done\|is up to date" "$HERE/run.log"; then
  echo "make built nothing"
  exit 1
fi
# Copy the results out (not timed), one `occlum run` each: no shell.
n=0
for d in "${SRCS[@]}"; do
  n=$((n + 1))
  occlum run /bin/busybox cp "$d/lua" "/host/lua.$n" >>"$HERE/run.log" 2>&1 || true
done
# Where e2e/lua-occlum.sh looks for them, whichever the instance.
rm -rf "$HERE/out" && mkdir -p "$HERE/out"
cp host/lua.* "$HERE/out/" 2>/dev/null || true
exit $rc
