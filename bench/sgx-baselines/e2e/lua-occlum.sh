#!/bin/bash
# Build Lua 5.5.1 inside Occlum (occlum/lua/inside.sh, in the Occlum
# container), then check the result on the host, as e2e/lua-gramine.sh does.
# Occlum runs every process of the build in one enclave (one address space,
# spawn/vfork, no fork); sources and objects are in its encrypted SEFS.
# Usage: e2e/lua-occlum.sh   (prints "wall <s> rc=<rc>"; TWICE=1, PROBE=1,
#        EDMM=1 for an enclave of TriSeal's size with EDMM: see inside.sh)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH=$(cd "$HERE/.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
docker run --rm --device /dev/sgx_enclave --device /dev/sgx_provision -v /dev/sgx:/dev/sgx \
  -v "$BENCH:/bench" -v "$E:$E" \
  -e LUA_E="$E" -e PROBE="${PROBE:-0}" -e TWICE="${TWICE:-0}" -e EDMM="${EDMM:-0}" \
  -e JOBS="${JOBS:-}" -e JOBSTYLE="${JOBSTYLE:-}" \
  $(for v in OCCLUM_USER_SPACE OCCLUM_USER_SPACE_MAX OCCLUM_KHEAP OCCLUM_KHEAP_MAX; do
      [ -n "${!v:-}" ] && echo "-e $v=${!v}"; done) \
  -w /bench/occlum/lua occlum/occlum:latest-ubuntu20.04 bash ./inside.sh
rc=$?
# The container ran as root: hand the files back.
docker run --rm -v "$BENCH/occlum/lua:/s" occlum/occlum:latest-ubuntu20.04 chown -R "$(id -u):$(id -g)" /s
[ "${PROBE:-0}" = 1 ] && exit $rc
n=1
[ "${TWICE:-0}" = 1 ] && n=2
for i in $(seq $n); do
  out=$BENCH/occlum/lua/out/lua.$i
  [ -f "$out" ] && "$out" -e 'print(_VERSION, 2^10, string.format("%q", "ok"))' || echo "lua: missing"
done
exit $rc
