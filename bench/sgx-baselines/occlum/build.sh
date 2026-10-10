#!/bin/bash
# Run inside occlum/occlum container (see run-docker.sh).
# Usage: ./build.sh <app>   e.g. ./build.sh forkexecv_spawn
# Occlum has no fork(), so everything is built with -DNO_FORK.
set -euo pipefail
APP=${1:-hello}
CC="occlum-gcc -O2 -DNO_FORK"
cd "$(dirname "$0")"
rm -rf inst && occlum new inst >/dev/null
$CC -o inst/image/bin/$APP ../src/$APP.c
mkdir -p inst/image/automated_tests
for h in hello nop; do
  $CC -o inst/image/automated_tests/$h ../src/$h.c
done
cd inst
# Keep resources in line with the Gramine/TriSeal configs, and put /tmp in
# enclave memory like Gramine's tmpfs and TriSeal's imfs.
jq '.resource_limits.user_space_size = "'${USER_SPACE:-600MB}'"
  | .resource_limits.max_num_of_threads = '${MAX_THREADS:-8}'
  | .mount = ([.mount[] | select(.target != "/tmp")] + [{"target": "/tmp", "type": "ramfs"}])' \
  Occlum.json > o.json && mv o.json Occlum.json
occlum build >/dev/null
echo "built $APP; run with: cd inst && occlum run /bin/$APP"
# The container runs as root; hand the build tree back to the host user.
chown -R "$(stat -c %u:%g ../../src)" .
