#!/bin/bash
# exectest.sh <workdir>: sealed repro of run1's last step. Full kaem from Step 4's tree runs one child,
# then execs kaem on after.kaem, with Blink logging to stderr.
set -uo pipefail
W=$1
E=$HOME/evid/e2e; S=$HOME/stage0-seal; T=$S/exectest/stage0; B=$S/bin; ENARX=$S/enarx
MKPRELOAD=$HOME/grates-pr/tools/mkpreload/target/release/mkpreload
rm -rf "$W"; mkdir -p "$W"
mapfile -t sources < <(find $T -type f)
BUNDLE=/dev/shm/stage0-exectest.$$.prld; DUMP=/dev/shm/stage0-exectest.$$.dump
$MKPRELOAD $BUNDLE "$B/cdrun" "$B/blink" --section "${sources[@]}" 2> $W/mkpreload.log || { tail -3 $W/mkpreload.log; exit 1; }
{
  printf 'args = ['; sep=""
  for a in "$E/grates/imfs-grate.cwasm" --preload-shm --attest --dump-shm "$B/cdrun" "$T" "$B/blink" -m -e ./AMD64/bin/kaem --verbose --strict --file ./test.kaem; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nTMPDIR = "/tmp"\nIMFS_RESERVE_MB = "512"\n\n'
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((256 << 20))
} > $W/build.toml
until ! pgrep -x enarx >/dev/null && ! pgrep -x loader >/dev/null; do sleep 15; done
start=$(date +%s)
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE=$W/build.toml timeout -s KILL 1200 $ENARX run $E/grates/intoto-grate.cwasm > $W/build.log 2>&1
rc=$?; end=$(date +%s)
echo "wall $((end - start)) rc=$rc"
rm -f $BUNDLE $DUMP
echo EXECTEST-DONE
