#!/bin/bash
# usage: stage0-run-amd64.sh <tree> [blink]
cd "$1" || exit 1
start=$(date +%s.%N)
if [ "$2" = blink ]; then
  ~/blink/o//blink/blink -m ./bootstrap-seeds/POSIX/AMD64/kaem-optional-seed
else
  ./bootstrap-seeds/POSIX/AMD64/kaem-optional-seed
fi
rc=$?
end=$(date +%s.%N)
echo "EXIT rc=$rc wall=$(echo "$end - $start" | bc) s"
sha256sum -c amd64.answers && echo "ANSWERS ALL OK"
