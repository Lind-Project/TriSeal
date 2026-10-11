#!/bin/bash
# cc1 alone on a preprocessed single-file program (<prog>.i, from gcc -E with
# the flags of sfp.conf), the same compile three ways, under measure.py:
#   native   the host's cc1 (gcc 15.2.0)
#   lind     the wasm cc1 under lind-boot, outside any TEE (LIND_BOOT: a
#            lind-boot built to run outside a keep, no chroot)
#   triseal  the wasm cc1 in a TriSeal keep, under the IMFS grate, with
#            cc1 and the input preloaded, the assembly dumped at exit
# Usage: e2e/sfp/cc1-alone.sh <native|lind|triseal> <prog>   (D=<dir with .i>)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH=$(cd "$HERE/../.." && pwd)
E=${E:-$HOME/sgx-bench/e2e}
D=${D:-$E/cc1-alone}
R=${ROOT:-$E/root}
mode=$1 p=$2
W=$D/work-$mode-$p
rm -rf "$W" && mkdir -p "$W"
# the input in the work directory, so that IMFS has the output's directory
cp "$D/$p.i" "$W/"
ARGS=(-fpreprocessed "$W/$p.i" -quiet -mtune=generic -march=x86-64 -O2 -w -std=gnu90 -fpermissive -o "$W/$p.s")
M=(python3 "$HERE/measure.py" "$W/measure")
case $mode in
  native) "${M[@]}" "$E/native-gcc/bin/cc1" "${ARGS[@]}" ;;
  lind) LIND_NO_CHROOT=1 "${M[@]}" "$LIND_BOOT" "$R/usr/local/bin/cc1" "${ARGS[@]}" ;;
  triseal)
    ENARX=${ENARX:-$E/ze/TriSeal/target/release/enarx}
    IMFS_GRATE=${IMFS_GRATE:-$BENCH/triseal/grates/ze/imfs-grate.cwasm}
    {
      printf 'args = ['
      sep=""
      for a in "$R/usr/local/bin/cc1" "${ARGS[@]}"; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
      printf ']\n\n[env]\nPRELOADS = "%s:%s"\nDUMPS = "%s=%s"\n' "$R/usr/local/bin/cc1" "$W/$p.i" "$W/$p.s" "$W/$p.s"
    } >"$W/run.toml"
    LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$W/run.toml" \
      "${M[@]}" "$ENARX" run "$IMFS_GRATE" ;;
esac >"$W/run.log" 2>&1
rc=$?
grep "^measure:" "$W/run.log"
echo "$p.s: $(wc -l <"$W/$p.s" 2>/dev/null || echo missing) lines"
exit $rc
