#!/bin/bash
# Host wrapper for Occlum experiment E (occlum/sec/inside-e.sh in the container).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"
BENCH=$(cd "$HERE/../.." && pwd)
out=$RESULTS/occlum-e.out
docker run --rm --device /dev/sgx_enclave --device /dev/sgx_provision -v /dev/sgx:/dev/sgx \
  -v "$BENCH:/bench" -w /bench/occlum/sec occlum/occlum:latest-ubuntu20.04 \
  bash ./inside-e.sh >"$out" 2>&1
docker run --rm -v "$BENCH/occlum:/s" occlum/occlum:latest-ubuntu20.04 chown -R "$(id -u):$(id -g)" /s >/dev/null 2>&1 || true
echo "=== Occlum E output ==="
grep -E "occlum-E|run rc|SUCCEEDED|DENIED|FAULTED|error|Error" "$out" | head -30
# Verdict for V1 (the architectural differentiator): shared address space?
if grep -q "V1 direct pointer read:.*SUCCEEDED(LEAK)" "$out"; then
  sec_log E occlum direct-pointer SUCCEEDED "one process read another's memory by plain pointer; Occlum build steps share one enclave address space (no intra-enclave memory isolation)"
elif grep -q "V1 direct pointer read:.*FAULTED" "$out"; then
  sec_log E occlum direct-pointer DENIED "cross-process pointer read faulted (address not mapped in reader)"
elif grep -q "V1 direct pointer read:.*DENIED" "$out"; then
  sec_log E occlum direct-pointer DENIED "cross-process pointer read returned no secret"
else
  sec_log E occlum direct-pointer ERROR "see occlum-e.out"
fi
for v in "V2 /proc" "V3 process_vm_readv"; do
  line=$(grep -m1 "$v" "$out")
  case "$line" in
    *"SUCCEEDED(LEAK)"*) sec_log E occlum "${v%% *}" SUCCEEDED "$line";;
    *DENIED*)            sec_log E occlum "${v%% *}" DENIED "$line";;
    *)                   sec_log E occlum "${v%% *}" ERROR "no line";;
  esac
done
