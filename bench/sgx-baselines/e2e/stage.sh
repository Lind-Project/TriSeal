#!/bin/bash
# Stage everything the end-to-end compile benchmarks need under $E
# (default ~/sgx-bench/e2e). Nothing outside $E is modified.
#
#   root/       the lind toolchain (gcc, cc1, as, ld, ar, ranlib, make from
#               lind-wasm-apps with PR 288, built in the $CONTAINER container
#               with lind-wasm#1397) precompiled for the host's lind-boot,
#               plus headers, crt files and glibc 2.35 from that container.
#               libc.so and libm.so are rewritten to point inside root/.
#   lua-5.5.1-src/  clean Lua sources
#   native-gcc/bin/ gcc 15.2.0 built natively from the same source, for the
#               native and Gramine runs (stripped driver and cc1)
#
# in-toto-cli (lind-wasm-apps PR 284) and TriSeal's enarx are built
# separately; see e2e/README.md.
set -euo pipefail
E=${E:-$HOME/sgx-bench/e2e}
CONTAINER=${CONTAINER:-lind-sep25}
CL=/home/lind/lind-wasm/lindfs
APPS=/home/lind/lind-wasm/lind-wasm-apps
LINDBOOT=${LINDBOOT:-$HOME/lind-wasm/build/lind-boot}
GCC_SRC=${GCC_SRC:-$HOME/lind-wasm-apps/gcc}
R=$E/root

mkdir -p "$E/wasm" "$R/usr/local/bin" "$R/usr/local/lib" "$R/usr/lib" "$R/lib"
for p in gcc/xgcc:gcc gcc/cc1:cc1 binutils/as:as binutils/ld:ld binutils/ar:ar \
         binutils/ranlib:ranlib make/make:make; do
  src=${p%%:*} dst=${p##*:}
  docker cp "$CONTAINER:$APPS/$src.opt.wasm" "$E/wasm/$dst.opt.wasm"
  "$LINDBOOT" --precompile "$E/wasm/$dst.opt.wasm" >/dev/null
  cp "$E/wasm/$dst.opt.cwasm" "$R/usr/local/bin/$dst"
done

docker cp "$CONTAINER:$CL/usr/include" "$R/usr/"
docker cp "$CONTAINER:$CL/usr/local/lib/gcc" "$R/usr/local/lib/"
docker cp "$CONTAINER:$CL/usr/lib/x86_64-linux-gnu" "$R/usr/lib/"
docker cp "$CONTAINER:$CL/lib/x86_64-linux-gnu" "$R/lib/"
docker cp "$CONTAINER:$CL/lib64" "$R/"

# The linker scripts name host paths; under enarx there is no chroot.
L=$R/usr/lib/x86_64-linux-gnu
cat >"$L/libc.so" <<EOT
/* Staged copy: points at this root instead of the host's glibc. */
OUTPUT_FORMAT(elf64-x86-64)
GROUP ( $R/lib/x86_64-linux-gnu/libc.so.6 $L/libc_nonshared.a  AS_NEEDED ( $R/lib64/ld-linux-x86-64.so.2 ) )
EOT
cat >"$L/libm.so" <<EOT
/* Staged copy: points at this root instead of the host's glibc. */
OUTPUT_FORMAT(elf64-x86-64)
GROUP ( $R/lib/x86_64-linux-gnu/libm.so.6  AS_NEEDED ( $R/lib/x86_64-linux-gnu/libmvec.so.1 ) )
EOT

rm -rf "$E/lua-5.5.1-src"
docker cp "$CONTAINER:$CL/lua-5.5.1" "$E/lua-5.5.1-src"
rm -f "$E"/lua-5.5.1-src/src/{*.o,*.a,lua,luac}

# Native gcc 15.2.0 with the same configure flags as lind-wasm-apps' build.
mkdir -p "$E/native-gcc-build" "$E/native-gcc/bin"
(cd "$E/native-gcc-build" &&
  "$GCC_SRC/configure" --target=x86_64-linux-gnu --disable-bootstrap --enable-languages=c \
    --disable-shared --enable-static --disable-threads --disable-multilib --disable-plugin \
    --disable-nls --disable-libgcc --disable-libssp --disable-libgomp --disable-libatomic \
    --disable-libquadmath --disable-libvtv --disable-libitm --disable-libsanitizer \
    --disable-libstdc++-v3 --disable-libffi --disable-decimal-float --disable-lto --disable-gcov \
    --enable-checking=release --without-headers --without-isl --with-gnu-as --with-gnu-ld \
    >configure.log 2>&1 &&
  make -j"$(nproc)" all-gcc >make.log 2>&1)
strip -o "$E/native-gcc/bin/gcc" "$E/native-gcc-build/gcc/xgcc"
strip -o "$E/native-gcc/bin/cc1" "$E/native-gcc-build/gcc/cc1"
echo "staged under $E"
