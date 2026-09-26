#!/bin/bash
# Start the Occlum dev container with the SGX devices and this bench tree mounted.
BENCH=$(cd "$(dirname "$0")/.." && pwd)
exec docker run $([ -t 0 ] && echo -it) --rm \
  --device /dev/sgx_enclave --device /dev/sgx_provision \
  -v /dev/sgx:/dev/sgx \
  -v "$BENCH:/bench" -w /bench/occlum \
  occlum/occlum:latest-ubuntu20.04 "$@"
