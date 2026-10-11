#!/bin/bash
# What lind's instrumentation costs a CPU-bound C program: oggenc.c built
# once with lind's clang (static, -O2), then lind-wasm-opt with its standard
# instrumentation (epoch injection for signals, asyncify for fork/exec) and
# without each, every variant precompiled and run by the same lind-boot
# outside any TEE, encoding the same audio; and oggenc built natively with
# gcc and clang -O2. Same source, same flags as sfp.conf where they apply.
# Usage: e2e/sfp/wasm-variants.sh      (LIND_BOOT=<lind-boot, no chroot>,
#        LIND_WASM=<lind-wasm checkout with its toolchain>, N=<runs>)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
E=${E:-$HOME/sgx-bench/e2e}
LIND_WASM=${LIND_WASM:-$HOME/lind-wasm}
N=${N:-3}
D=${D:-$E/wasm-variants}
rm -rf "$D" && mkdir -p "$D"
cd "$D" || exit 1
cp "$SFP/src/oggenc.c" .
FLAGS=(-std=gnu89 -w)

# input: 10 minutes of stereo 44.1 kHz 16-bit audio (a deterministic minute,
# ten times)
python3 - <<'PY'
import math, struct, wave
w = wave.open("in.wav", "wb"); w.setnchannels(2); w.setsampwidth(2); w.setframerate(44100)
frames = bytearray()
x = 12345
for i in range(44100 * 60):
    x = (1103515245 * x + 12345) & 0x7fffffff
    a = int(8000 * math.sin(i * 0.0313) + 3000 * math.sin(i * 0.217) + (x >> 18) - 4096)
    b = int(7000 * math.sin(i * 0.0271) + 2500 * math.sin(i * 0.191) + (x >> 19) - 2048)
    frames += struct.pack("<hh", a, b)
w.writeframes(bytes(frames) * 10); w.close()
PY

gcc -O2 "${FLAGS[@]}" -fpermissive oggenc.c -o native-gcc -lm
clang -O2 "${FLAGS[@]}" -Wno-error oggenc.c -o native-clang -lm 2>clang.log || echo "native clang build failed" >&2
"$LIND_WASM/scripts/lind_compile" --compile-only -s oggenc.c -- "${FLAGS[@]}" -lm >compile.log 2>&1 ||
  { tail -5 compile.log; exit 1; }
base=$(ls -t ./*.wasm 2>/dev/null | head -1)
[ -n "$base" ] || base=$(find "$LIND_WASM/lindfs" -name oggenc.wasm -newer oggenc.c | head -1)
cp "$base" plain.wasm
OPT=$LIND_WASM/scripts/lind-wasm-opt
"$OPT" --static plain.wasm -o default.wasm
"$OPT" --static --without-asyncify plain.wasm -o no-asyncify.wasm
"$OPT" --static --without-epoch plain.wasm -o no-epoch.wasm
"$OPT" --static --without-asyncify --without-epoch plain.wasm -o neither.wasm
# lind needs the epoch instrumentation in a main module (no-epoch and
# neither do not start); without asyncify the program runs to the end and
# lind panics at exit, which unwinds through asyncify: the output is
# complete and checked like the others.
for v in default no-asyncify no-epoch neither; do
  "$LIND_BOOT" --precompile "$v.wasm" >/dev/null 2>&1
  [ -f "$v.cwasm" ] || echo "precompile of $v failed" >&2
done
ls -l ./*.wasm ./*.cwasm native-* | awk '{print $5, $9}'

run() {
  local name=$1; shift
  for i in $(seq "$N"); do
    rm -f out.ogg
    local s e
    s=$(date +%s.%N)
    "$@" -Q -o "$D/out.ogg" "$D/in.wav" >/dev/null 2>&1
    e=$(date +%s.%N)
    echo "$name,$i,$(echo "$e - $s" | bc),$(stat -c %s out.ogg 2>/dev/null || echo 0)"
  done
}
echo "variant,run,seconds,ogg bytes" | tee results.csv
{
  run native-gcc ./native-gcc
  [ -x native-clang ] && run native-clang ./native-clang
  for v in default no-asyncify; do
    run "lind-$v" env LIND_NO_CHROOT=1 "$LIND_BOOT" "$v.cwasm"
  done
} | tee -a results.csv
