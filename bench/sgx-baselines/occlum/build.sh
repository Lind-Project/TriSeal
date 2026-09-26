#!/bin/bash
# Run inside occlum/occlum container (see run-docker.sh).
# Usage: ./build.sh <app>   e.g. ./build.sh forkexecv_spawn
set -euo pipefail
APP=${1:-hello}
cd "$(dirname "$0")"
rm -rf inst && occlum new inst >/dev/null
occlum-gcc -O2 -o inst/image/bin/$APP ../src/$APP.c
mkdir -p inst/image/automated_tests
occlum-gcc -O2 -o inst/image/automated_tests/hello ../src/hello.c
cd inst
# Keep resources in line with the Gramine/TriSeal configs.
jq '.resource_limits.user_space_size = "'${USER_SPACE:-600MB}'"
  | .resource_limits.max_num_of_threads = '${MAX_THREADS:-8}'' Occlum.json > o.json && mv o.json Occlum.json
occlum build >/dev/null
echo "built $APP; run with: cd inst && occlum run /bin/$APP"
# The container runs as root; hand the build tree back to the host user.
chown -R "$(stat -c %u:%g ../../src)" .
