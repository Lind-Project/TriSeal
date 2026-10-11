#!/bin/bash
# Build one single-file program inside Occlum (occlum/sfp/inside.sh, in the
# Occlum container), then smoke-test the result on the host.
# Usage: e2e/sfp/occlum.sh <prog>   (prints "wall <s>"; PROBE=1, STRACE=1)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
BENCH=$(cd "$HERE/../.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
p=$1
docker run --rm --device /dev/sgx_enclave --device /dev/sgx_provision -v /dev/sgx:/dev/sgx \
  -v "$BENCH:/bench" -v "$E:$E" -v "$SFP:$SFP" \
  -v /usr/bin/strace:/hoststrace/strace:ro -v /usr/lib/x86_64-linux-gnu:/hoststrace/lib:ro \
  -e SFP="$SFP" -e SFP_E="$E" -e PROBE="${PROBE:-0}" -e STRACE="${STRACE:-0}" \
  -w /bench/occlum/sfp occlum/occlum:latest-ubuntu20.04 bash ./inside.sh "$p"
rc=$?
# The container ran as root: hand the files back.
docker run --rm -v "$BENCH/occlum/sfp:/s" occlum/occlum:latest-ubuntu20.04 chown -R "$(id -u):$(id -g)" /s
[ "${PROBE:-0}" = 1 ] && exit $rc
"$HERE/check.sh" "$p" "$BENCH/occlum/sfp/inst-$p/host/$(sfp_out "$p")"
exit $rc
