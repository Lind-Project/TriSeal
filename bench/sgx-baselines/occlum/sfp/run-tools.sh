#!/bin/bash
# Start the Occlum container with this bench tree at /bench and the
# lind-wasm-apps sources read-only at /apps (for build-tools.sh).
BENCH=$(cd "$(dirname "$0")/../.." && pwd)
exec docker run $([ -t 0 ] && echo -it) --rm \
  --device /dev/sgx_enclave --device /dev/sgx_provision \
  -v /dev/sgx:/dev/sgx \
  -v "$BENCH:/bench" -v "$HOME/lind-wasm-apps:/apps:ro" -v "$HOME/sgx-bench/e2e:$HOME/sgx-bench/e2e" \
  -v "$HOME/sgx-bench/sfp:$HOME/sgx-bench/sfp" -w /bench/occlum/sfp \
  occlum/occlum:latest-ubuntu20.04 "$@"
