#!/bin/bash
# Run inside the Occlum container (e2e/sfp/occlum.sh starts it): build one
# single-file program with make + gcc 15.2.0 (sfp/build-tools.sh) inside an
# Occlum enclave, like e2e/sfp/{triseal,gramine}.sh do in theirs.
#   inst-<prog>/ is made once: the tools (/bin/{make,gcc,cc1}, the
#   container's as and ld, busybox for sh and cp), the staged sysroot and the
#   program's tree, all at the same paths as on the host, in Occlum's
#   encrypted root file system (SEFS); /tmp is in enclave memory (ramfs) and
#   /host is a host directory the result is copied to.
# Usage: inside.sh <prog>   (prints "wall <s>" for `occlum run`)
#   PROBE=1   run `make --version` instead
#   STRACE=1  count the host syscalls of `occlum run` (strace -f -c, the
#             host's strace through the host's loader) into strace.<kind>.txt
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. /bench/e2e/sfp/sfp.conf
E=$SFP_E
R=$E/root
G=$R/usr/local/lib/gcc/x86_64-linux-gnu/15.2.0
L=$R/usr/lib/x86_64-linux-gnu
p=$1
W=$E/work-sfp-occlum/$p
I=$HERE/inst-$p
out=$(sfp_out "$p")

if [ ! -f "$I/.built" ]; then
  rm -rf "$I"
  mkdir -p "$HERE/tools-strip"
  for t in gcc cc1 make; do
    [ -f "$HERE/tools-strip/$t" ] || strip -o "$HERE/tools-strip/$t" "$HERE/tools/bin/$t"
  done
  rm -rf "$HERE/stage-$p" && mkdir -p "$HERE/stage-$p"
  cp -r "$SFP/tree/$p" "$HERE/stage-$p/$p"
  cat >"$HERE/sfp-$p.yaml" <<YAML
includes:
  - base.yaml
targets:
  - target: /bin
    copy:
      - files:
        - $HERE/tools-strip/gcc
        - $HERE/tools-strip/cc1
        - $HERE/tools-strip/make
        - /opt/occlum/toolchains/busybox/glibc/busybox
  - target: /usr/bin
    copy:
      - files:
        - /usr/bin/x86_64-linux-gnu-as
        - /usr/bin/x86_64-linux-gnu-ld.bfd
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
  - target: $E/work-sfp-occlum
    copy:
      - dirs:
        - $HERE/stage-$p/$p
YAML
  occlum new "$I" >/dev/null
  cd "$I"
  rm -rf image
  copy_bom -f "$HERE/sfp-$p.yaml" --root image --include-dir /opt/occlum/etc/template >copy_bom.log 2>&1
  # as and ld also in /bin, a -B prefix, so that gcc starts them by full
  # path: starting them through a PATH search (posix_spawnp) fails in Occlum.
  ln -sf x86_64-linux-gnu-as image/usr/bin/as
  ln -sf x86_64-linux-gnu-ld.bfd image/usr/bin/ld
  ln -sf ../usr/bin/x86_64-linux-gnu-as image/bin/as
  ln -sf ../usr/bin/x86_64-linux-gnu-ld.bfd image/bin/ld
  # As much user memory as the build needs (cc1 on gcc.c peaks at 1.55 GB
  # natively), /tmp in enclave memory, /host for the result.
  us=1GB
  [ "$p" = gcc ] && us=${OCCLUM_USER_SPACE_gcc:-3GB}
  jq '.resource_limits.user_space_size = "'$us'"
    | .resource_limits.max_num_of_threads = 16
    | .resource_limits.kernel_space_heap_size = "256MB"
    | .process.default_stack_size = "8MB"
    | .env.default = ((.env.default // []) + ["PATH=/bin:/usr/bin"])
    | .mount = ([.mount[] | select(.target != "/tmp" and .target != "/host")]
        + [{"target": "/tmp", "type": "ramfs"}, {"target": "/host", "type": "hostfs", "source": "./host"}])' \
    Occlum.json >o.json && mv o.json Occlum.json
  mkdir -p host
  occlum build >build.log 2>&1
  touch .built
fi

cd "$I"
rm -f "host/$out"
# SEFS keeps what a run writes: remove the last build's output first (not
# timed), or make finds it up to date and builds nothing.
if [ "${PROBE:-0}" != 1 ]; then
  occlum run /bin/busybox rm -f "$W/$out" >"$HERE/rm.$p.log" 2>&1
fi
CC="/bin/gcc -B/bin/ -B$L/ -B$G/ -isystem $G/include -isystem $R/usr/include"
# No shell: Occlum has no fork, and make runs its one-line recipe itself.
if [ "${PROBE:-0}" = 1 ]; then
  kind=probe
  RUN=(occlum run /bin/make --version)
else
  kind=build
  RUN=(occlum run /bin/make -C "$W" "CC=$CC" "CFLAGS=$SFP_CFLAGS" "LIBS=$(sfp_libs "$p")")
fi
if [ "${STRACE:-0}" = 1 ]; then
  RUN=(/hoststrace/lib/ld-linux-x86-64.so.2 --library-path /hoststrace/lib /hoststrace/strace
       -f -c -o "$HERE/strace.$p.$kind.txt" "${RUN[@]}")
fi
start=$(date +%s%N)
set +e
"${RUN[@]}" >"$HERE/run.$p.log" 2>&1
rc=$?
set -e
end=$(date +%s%N)
ns=$((end - start))
printf 'wall %d.%09d rc=%d\n' $((ns / 1000000000)) $((ns % 1000000000)) "$rc"
grep -E "error|Error|undefined" "$HERE/run.$p.log" | head -5 || true
if grep -q "is up to date" "$HERE/run.$p.log"; then
  echo "make built nothing"
  exit 1
fi
# Copy the result out (not timed): a second run, since there is no shell.
if [ "$kind" = build ] && [ "$rc" = 0 ]; then
  occlum run /bin/busybox cp "$W/$out" "/host/$out" >>"$HERE/run.$p.log" 2>&1 || true
fi
exit $rc
