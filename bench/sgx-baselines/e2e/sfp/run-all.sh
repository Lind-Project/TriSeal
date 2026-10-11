#!/bin/bash
# All the single-file-program benchmarks, into one directory:
#   coldwarm/     e2e/sfp/coldwarm.sh (cold, startup, first, warm)
#   after-start/  e2e/sfp/after-start.sh (what each build asks of the host)
# Usage: e2e/sfp/run-all.sh [outdir]
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
OUT=${1:-$E/sfp-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
{
  date
  uname -a
  for f in "${ENARX:-$E/ze/TriSeal/target/release/enarx}" "${ENARX_STATS:-$E/ze/TriSeal-s/target/release/enarx}" \
    "${IMFS_GRATE:-$HERE/../../triseal/grates/ze/imfs-grate.cwasm}" \
    "${INTOTO_GRATE:-$HERE/../../triseal/grates/ze/intoto-grate.cwasm}"; do
    echo "$f $(stat -c %y "$f")"
  done
  echo "toolchain ${ROOT:-$E/root}"
  gramine-sgx --version 2>/dev/null | head -1
  docker image inspect --format "occlum image {{.Id}} {{.Created}}" occlum/occlum:latest-ubuntu20.04
  cat "${SFP:-$HOME/sgx-bench/sfp}/src/SHA256SUMS"
} >"$OUT/versions.txt" 2>&1
echo "== coldwarm $(date +%T)"
OUT=$OUT/coldwarm "$HERE/coldwarm.sh" 2>"$OUT/coldwarm.progress"
echo "   rc=$? $(date +%T)"
echo "== after-start $(date +%T)"
OUT=$OUT/after-start "$HERE/after-start.sh" 2>"$OUT/after-start.progress"
echo "   rc=$? $(date +%T)"
echo "results: $OUT"
