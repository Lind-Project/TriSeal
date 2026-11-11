#!/bin/bash
# usage: stage0-run.sh <tree> <log> [strace]
cd "$1" || exit 1
start=$(date +%s.%N)
if [ "$3" = strace ]; then
  strace -f -c -o "$2.strace" ./bootstrap-seeds/POSIX/x86/kaem-optional-seed
else
  ./bootstrap-seeds/POSIX/x86/kaem-optional-seed
fi
rc=$?
end=$(date +%s.%N)
echo "EXIT rc=$rc wall=$(echo "$end - $start" | bc) s"
sha256sum -c x86.answers && echo "ANSWERS ALL OK"
