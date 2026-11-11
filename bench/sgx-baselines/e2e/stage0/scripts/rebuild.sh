#!/bin/bash
# Rebuild lind Blink in lind-sep25, check hex0 in plain lind, precompile for the full setup, rebuild the stage0 enarx.
set -eu
LB=$HOME/mkj/target-lb-t12/release/lind-boot; S=$HOME/stage0-seal
docker exec lind-sep25 bash -c "cd /home/lind/blink-wasm && make -j16 o//blink/blink > make-wasm3.log 2>&1 && cp o//blink/blink blink.wasm && /home/lind/lind-wasm/scripts/bin/lind-wasm-opt --static blink.wasm -o blink.opt.wasm && /home/lind/lind-wasm/build/lind-boot --precompile blink.opt.wasm && sudo cp blink.opt.cwasm /home/lind/lind-wasm/lindfs/blinktest/blink.cwasm && cd /home/lind/lind-wasm && sudo rm -f lindfs/blinktest/hex0-out && sudo ./scripts/bin/lind_run /blinktest/blink.cwasm -m /blinktest/hex0-seed /blinktest/hex0_AMD64.hex0 /blinktest/hex0-out && sudo sha256sum lindfs/blinktest/hex0-out"
docker cp lind-sep25:/home/lind/blink-wasm/blink.opt.wasm $S/wasm/blink.opt.wasm
(cd $S/wasm && $LB --precompile --explicit-bounds-checks blink.opt.wasm && cp blink.opt.cwasm ../bin/blink)
sha256sum $S/bin/*
$S/setup.sh
echo REBUILD-DONE
