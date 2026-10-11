#!/bin/bash
# Run inside the occlum/occlum:latest-ubuntu20.04 container (sfp/run-tools.sh):
# build the toolchain the single-file benchmarks use on the other systems,
# from the same sources (lind-wasm-apps, mounted at /apps), against the
# container's glibc 2.31, which is Occlum's glibc:
#   gcc 15.2.0  driver + cc1, configured like e2e/native-gcc (Gramine's),
#               plus --enable-host-pie: Occlum loads only PIE executables
#   as, ld: the container's binutils 2.34 (building binutils 2.46 needs
#           flex and bison, which the container does not have)
#   make 4.4  --disable-posix-spawn, so that it starts commands with vfork:
#             Occlum refuses glibc's posix_spawn (clone with CLONE_VFORK) but
#             has vfork (as for Gramine, whose make is built the same way)
# Installs into /bench/occlum/sfp/tools. (No makeinfo in the container: the
# documentation is skipped with MAKEINFO=true.)
set -euo pipefail
OUT=/bench/occlum/sfp/tools
J=$(nproc)
B=/bench/occlum/sfp/build
rm -rf "$B" "$OUT"
mkdir -p "$B"/{gcc,make} "$OUT"

cd "$B/gcc"
/apps/gcc/configure --prefix="$OUT" --target=x86_64-linux-gnu --disable-bootstrap --enable-languages=c \
  --disable-shared --enable-static --disable-threads --disable-multilib --disable-plugin --disable-nls \
  --disable-libgcc --disable-libssp --disable-libgomp --disable-libatomic --disable-libquadmath \
  --disable-libvtv --disable-libitm --disable-libsanitizer --disable-libstdc++-v3 --disable-libffi \
  --disable-decimal-float --disable-lto --disable-gcov --enable-checking=release --without-headers \
  --without-isl --with-gnu-as --with-gnu-ld --enable-host-pie >configure.log 2>&1
make -j"$J" MAKEINFO=true all-gcc >make.log 2>&1
mkdir -p "$OUT/bin"
cp gcc/xgcc "$OUT/bin/gcc"
cp gcc/cc1 "$OUT/bin/cc1"

cd "$B/make"
/apps/make/configure --prefix="$OUT" --disable-nls --disable-posix-spawn >configure.log 2>&1
make -j"$J" MAKEINFO=true >make.log 2>&1
cp make "$OUT/bin/make"

"$OUT/bin/gcc" --version | head -1
as --version | head -1
"$OUT/bin/make" --version | head -1
