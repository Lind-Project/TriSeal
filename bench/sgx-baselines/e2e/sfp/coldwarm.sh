#!/bin/bash
# Cold vs warm builds of the single-file programs (oggenc, gcc) on
# native, TriSeal, Gramine (encrypted and plain files) and Occlum, as
# e2e/coldwarm.sh does for Lua:
#   cold     one build, from launching the TEE until it exits
#   startup  launching the TEE and running `make --version` (PROBE=1)
#   first    the first of two builds in one TEE instance (TWICE=1)
#   warm     the second one (module cache and IMFS warm on TriSeal)
# Occlum has no fork, so no twobuild there: cold and startup only.
# triseal-shm (triseal-shm.sh: preloads from shared memory, attested, output
# dumped to shared memory) has no twobuild either: cold and startup.
# Usage: e2e/sfp/coldwarm.sh            (OUT=<dir>, N=<runs>,
#        N_gcc=<runs for gcc.c>, SYSTEMS="native triseal ...")
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
E=${E:-$HOME/sgx-bench/e2e}
OUT=${OUT:-$E/sfp-coldwarm}
N=${N:-3}
SYSTEMS=${SYSTEMS:-native triseal triseal-shm gramine-encrypted gramine-allowed occlum}
mkdir -p "$OUT"
CSV=$OUT/coldwarm.csv
[ -f "$CSV" ] || echo "system,prog,run,kind,seconds,ok" >"$CSV"

# enclave size Gramine needs for each program (set after native runs)
genclave() { case $1 in gcc) echo "${GRAMINE_ENCLAVE_gcc:-4G}" ;; *) echo "${GRAMINE_ENCLAVE:-1G}" ;; esac; }

one() {
  local sys=$1 p=$2 i=$3 kind=$4
  shift 4
  local f=$OUT/$sys.$p.$kind.$i.out
  echo "== $sys $p $kind $i $(date +%T)" >&2
  "$@" >"$f" 2>&1
  local wall ok
  wall=$(sed -n 's/^wall \([0-9.]*\).*/\1/p' "$f" | tail -1)
  ok=$(grep -c ": ok" "$f")
  case $kind in
    twice)
      echo "$sys,$p,$i,twice-total,$wall,$ok" >>"$CSV"
      grep "^@build" "$f" | while read -r _ n _ s rc; do
        k=first; [ "$n" = 2 ] && k=warm
        echo "$sys,$p,$i,$k,$s,$([ "$rc" = 0 ] && echo 1 || echo 0)" >>"$CSV"
      done ;;
    *) echo "$sys,$p,$i,$kind,$wall,$ok" >>"$CSV" ;;
  esac
}

for p in "${SFP_PROGS[@]}"; do
  n=$N
  [ "$p" = gcc ] && n=${N_gcc:-$N}
  for i in $(seq "$n"); do
    for sys in $SYSTEMS; do
      case $sys in
        native)
          one native "$p" "$i" cold "$HERE/native.sh" "$p" "$E/work-sfp-native"
          one native "$p" "$i" twice env TWICE=1 "$HERE/native.sh" "$p" "$E/work-sfp-native" ;;
        triseal)
          one triseal "$p" "$i" cold "$HERE/triseal.sh" "$p" "$E/work-sfp-triseal"
          one triseal "$p" "$i" startup env PROBE=1 "$HERE/triseal.sh" "$p" "$E/work-sfp-triseal"
          one triseal "$p" "$i" twice env TWICE=1 "$HERE/triseal.sh" "$p" "$E/work-sfp-triseal" ;;
        triseal-shm)
          one triseal-shm "$p" "$i" cold "$HERE/triseal-shm.sh" "$p" "$E/work-sfp-triseal-shm"
          one triseal-shm "$p" "$i" startup env PROBE=1 "$HERE/triseal-shm.sh" "$p" "$E/work-sfp-triseal-shm" ;;
        gramine-*)
          m=${sys#gramine-}
          one "$sys" "$p" "$i" cold env MODE="$m" ENCLAVE_SIZE="$(genclave "$p")" "$HERE/gramine.sh" "$p" "$E/work-sfp-gramine"
          one "$sys" "$p" "$i" startup env MODE="$m" PROBE=1 ENCLAVE_SIZE="$(genclave "$p")" "$HERE/gramine.sh" "$p" "$E/work-sfp-gramine"
          one "$sys" "$p" "$i" twice env MODE="$m" TWICE=1 ENCLAVE_SIZE="$(genclave "$p")" "$HERE/gramine.sh" "$p" "$E/work-sfp-gramine" ;;
        occlum)
          [ -x "$HERE/occlum.sh" ] || continue
          one occlum "$p" "$i" cold "$HERE/occlum.sh" "$p"
          one occlum "$p" "$i" startup env PROBE=1 "$HERE/occlum.sh" "$p" ;;
      esac
    done
  done
done
echo "results: $CSV"
