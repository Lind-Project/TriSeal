#!/bin/bash
# Build an enarx for the stage0-posix sealed run: the current full-setup runtime (~/mkj/t12/evid, MAX_CAGEID 8192)
# with lind Blink and cdrun added to a copy of its allowlist. Leaves ~/evid/e2e/enarx and its target dir untouched.
set -eu
export PATH=$HOME/.cargo/bin:$PATH
M=$HOME/mkj; V=$M/t12/evid; S=$HOME/stage0-seal
cp $HOME/evid/e2e/allowlist.txt $S/allowlist.txt
(cd $S/bin && sha256sum blink cdrun | sed "s/  /  stage0-/") >> $S/allowlist.txt
tail -3 $S/allowlist.txt
[ -d $M/target-t12-stage0 ] || cp -a $M/target-t12-cageid $M/target-t12-stage0
s=$(date +%s)
(cd $V/TriSeal && ENARX_CWASM_ALLOWLIST=$S/allowlist.txt CARGO_TARGET_DIR=$M/target-t12-stage0 cargo build --release --bin enarx > $S/build-enarx.log 2>&1)
echo "enarx built in $(( $(date +%s)-s ))s"
cp $M/target-t12-stage0/release/enarx $S/enarx
ls -la $S/enarx
echo STAGE0-SETUP-DONE
