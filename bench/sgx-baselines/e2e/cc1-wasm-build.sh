#!/bin/bash
# Build cc1 for wasm32 as lind-wasm-apps/gcc/compile_gcc.sh does (same
# toolchain, sysroot, configure arguments and link flags), in its own build
# directory, with CFLAGS/CXXFLAGS = $1 (-Os as the script, or -O2). Writes
# nothing outside /home/lind/cc1-opt-ab. Runs in the lind-wasm-apps dev
# container (lind-sep25 on lind-server-3), as user lind; with -Os it rebuilds
# the toolchain's cc1.wasm byte for byte.
# Usage (in the container): cc1-wasm-build.sh -Os|-O2
set -euo pipefail
OPT=$1
APPS=/home/lind/lind-wasm/lind-wasm-apps
GCC_SRC=$APPS/gcc
MERGED_SYSROOT=$APPS/build/sysroot_merged
. "$APPS/build/.toolchain.env"
CLANGXX="$(dirname "$CLANG")/clang++"
LIBCXX_INCLUDE="$MERGED_SYSROOT/include/wasm32-wasi/c++/v1"
CC_WASM="$CLANG --target=wasm32-unknown-wasi --sysroot=$MERGED_SYSROOT \
  -pthread -matomics -mbulk-memory \
  -I$MERGED_SYSROOT/include -I$MERGED_SYSROOT/include/wasm32-wasi"
CXX_WASM="$CLANGXX --target=wasm32-unknown-wasi --sysroot=$MERGED_SYSROOT \
  -pthread -matomics -mbulk-memory \
  -stdlib=libc++ -isystem $LIBCXX_INCLUDE -fno-exceptions \
  -I$MERGED_SYSROOT/include -I$MERGED_SYSROOT/include/wasm32-wasi"
LDFLAGS_WASM="-Wl,--import-memory,--export-memory,--max-memory=67108864 \
  -Wl,-z,stack-size=8388608 \
  -Wl,--export=__stack_pointer,--export=__stack_low,--export=__tls_base \
  -L$MERGED_SYSROOT/lib/wasm32-wasi \
  -L$MERGED_SYSROOT/usr/lib/wasm32-wasi \
  -lc++ -lc++abi"
B=/home/lind/cc1-opt-ab/build$OPT
rm -rf "$B" && mkdir -p "$B" && cd "$B"
printf 'mpfr_cv_want_float128=no\n' > config.site
export CONFIG_SITE=$B/config.site
"$GCC_SRC/configure" \
  --build="$(gcc -dumpmachine)" --host=wasm32-unknown-wasi --target=x86_64-linux-gnu \
  --disable-bootstrap --enable-languages=c --disable-shared --enable-static \
  --disable-threads --disable-multilib --disable-plugin --disable-nls --disable-libgcc \
  --disable-libssp --disable-libgomp --disable-libatomic --disable-libquadmath \
  --disable-libvtv --disable-libitm --disable-libsanitizer --disable-libstdc++-v3 \
  --disable-libffi --disable-decimal-float --disable-lto --disable-gcov \
  --enable-checking=release --without-headers --without-isl --with-gnu-as --with-gnu-ld \
  CC_FOR_BUILD=gcc CXX_FOR_BUILD=g++ AR_FOR_BUILD=ar \
  CFLAGS_FOR_BUILD="-O2 -g" CXXFLAGS_FOR_BUILD="-O2 -g" LDFLAGS_FOR_BUILD="" \
  CC="$CC_WASM" CXX="$CXX_WASM" AR="$AR" NM="$NM" RANLIB="$RANLIB" \
  CFLAGS="$OPT" CXXFLAGS="$OPT" LDFLAGS="$LDFLAGS_WASM" > configure.log 2>&1
s=$(date +%s)
make all-gcc -j16 > make.log 2>&1
cp gcc/cc1 "/home/lind/cc1-opt-ab/cc1$OPT.wasm"
echo "built cc1$OPT.wasm $(stat -c %s gcc/cc1) bytes in $(( $(date +%s) - s )) s"
