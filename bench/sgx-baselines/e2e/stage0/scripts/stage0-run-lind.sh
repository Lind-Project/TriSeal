#!/bin/bash
# Step 4: stage0-posix AMD64 under lind Blink in container lind-sep25, outside SGX.
start=$(date +%s.%N)
docker exec lind-sep25 bash -c "cd /home/lind/lind-wasm && sudo ./scripts/bin/lind_run /blinktest/cdrun.cwasm /stage0 /blinktest/blink.cwasm -m ./bootstrap-seeds/POSIX/AMD64/kaem-optional-seed"
rc=$?
end=$(date +%s.%N)
echo "EXIT rc=$rc wall=$(echo "$end - $start" | bc) s"
docker exec lind-sep25 bash -c "cd /home/lind/lind-wasm/lindfs/stage0 && sudo sha256sum -c amd64.answers" && echo "ANSWERS ALL OK"
