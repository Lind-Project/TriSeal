#!/bin/bash
# stage0-evid.sh <workdir>: run the stage0-posix AMD64 bootstrap sealed in TriSeal's full setup, as strip-evid.sh does
# for strip: the in-toto grate runs IMFS, which stages the bundle from shared memory and has each archive attested,
# cdrun enters the tree and execs lind Blink, Blink runs the seed and every later x86_64 program, the 19 outputs in
# amd64.answers are dumped and signed, and verify-attest.py checks the evidence against the bundle and the dump.
set -uo pipefail
W=$1
E=$HOME/evid/e2e; S=$HOME/stage0-seal; T=$S/src; B=$S/bin; ENARX=$S/enarx
MKPRELOAD=$HOME/grates-pr/tools/mkpreload/target/release/mkpreload
rm -rf "$W"; mkdir -p "$W"
pipeline=("$B/cdrun" "$B/blink")
mapfile -t sources < <(sed "s#^#$T/#" $S/files.txt)
mapfile -t outs < <(awk '{print $2}' $T/amd64.answers)
dumps=""; for o in "${outs[@]}"; do dumps+="$T/$o=out/${o##*/};"; done
BUNDLE=/dev/shm/stage0-evid.$$.prld; DUMP=/dev/shm/stage0-evid.$$.dump
$MKPRELOAD $BUNDLE "${pipeline[@]}" --section "${sources[@]}" 2> $W/mkpreload.log || { tail -3 $W/mkpreload.log; exit 1; }
grep "^mkpreload:" $W/mkpreload.log
{
  printf 'args = ['; sep=""
  for a in "$E/grates/imfs-grate.cwasm" --preload-shm --attest --dump-shm "$B/cdrun" "$T" "$B/blink" ${BLINKFLAGS:--m} ./bootstrap-seeds/POSIX/AMD64/kaem-optional-seed; do printf '%s"%s"' "$sep" "$a"; sep=", "; done
  printf ']\n\n[env]\nTMPDIR = "/tmp"\nIMFS_RESERVE_MB = "%s"\nDUMPS = "%s"\n\n' "${RESERVE:-512}" "${dumps%;}"
  printf '[shm]\npreload = "%s"\ndump = "%s"\ndump_size = %d\n' "$BUNDLE" "$DUMP" $((256 << 20))
} > $W/build.toml
until ! pgrep -x enarx >/dev/null && ! pgrep -x loader >/dev/null; do sleep 15; done
start=$(date +%s)
LIND_GRATE_WORKERS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE=$W/build.toml python3 $HOME/evid/console_peek.py $W/console.log ${PEEK:-60} -- timeout -s KILL ${TO:-36000} $ENARX run $E/grates/intoto-grate.cwasm > $W/build.log 2>&1
rc=$?; end=$(date +%s)
echo "wall $((end - start)) rc=$rc"
grep -aE "error:|panicked|imfs-grate:" $W/build.log | grep -avE "^\[fork|wasm cache" | head -5
grep -a "^\[intoto-grate\]" $W/build.log
if [ -s $DUMP ]; then
  cp $DUMP $W/dump.prld
  $MKPRELOAD unpack $W/dump.prld $W/dump 2>&1 | tail -1
  (cd $W/dump/out && awk '{n=$2; sub(/.*\//, "", n); print $1 "  " n}' $T/amd64.answers | sha256sum -c) && echo "ANSWERS ALL OK"
  python3 $E/verify-attest.py $W/dump $BUNDLE
fi
cp $BUNDLE $W/bundle.prld 2>/dev/null
rm -f $BUNDLE $DUMP
echo STAGE0-EVID-DONE
