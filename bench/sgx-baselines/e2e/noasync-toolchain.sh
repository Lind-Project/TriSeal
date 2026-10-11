#!/bin/bash
# Stage $E/root-na: the toolchain of $E/root with cc1, as and ld built
# without Asyncify (they never fork, exec or start threads), for a runtime
# whose lind lets such a cage exit (lind-wasm exit-without-asyncify).
#
# The raw wasm comes from the lind-wasm-apps build in the dev container
# (CONTAINER, default lind-sep25); each is optimized exactly as its build
# script does it, only without --asyncify:
#   cc1     gcc/compile_gcc.sh: --epoch-injection --asyncify
#           --pass-arg=asyncify-removelist@@asyncify_ignore.txt --strip-debug -Os
#   as, ld  lind-wasm-opt --static (--epoch-injection --asyncify -O2 ...)
# The script first checks that these commands reproduce the toolchain's
# .opt.wasm byte for byte, then precompiles with the lind-boot that made its
# .cwasm files (LIND_BOOT).
# CC1_OPT (default -Os, as compile_gcc.sh) picks wasm-opt's level for cc1
# without Asyncify; with another level the tree is $E/root-na$CC1_OPT.
# Usage: e2e/noasync-toolchain.sh     (CC1_OPT=-O3 for $E/root-na-O3)
set -euo pipefail
E=${E:-$HOME/sgx-bench/e2e}
CONTAINER=${CONTAINER:-lind-sep25}
APPS=/home/lind/lind-wasm/lind-wasm-apps
LIND_WASM=${LIND_WASM:-$HOME/lind-wasm}
LIND_BOOT=${LIND_BOOT:-$LIND_WASM/build/lind-boot}
WO=$LIND_WASM/tools/binaryen/bin/wasm-opt
OPT=$LIND_WASM/scripts/lind-wasm-opt
D=$E/noasync
CC1_OPT=${CC1_OPT:--Os}
RNA=$E/root-na
[ "$CC1_OPT" = -Os ] || RNA=$E/root-na$CC1_OPT
mkdir -p "$D/raw"
cd "$D"

for f in gcc/cc1.wasm gcc/cc1.opt.wasm gcc/asyncify_ignore.txt binutils/as.wasm binutils/as.opt.wasm \
  binutils/ld.wasm binutils/ld.opt.wasm; do
  [ -f "raw/$(basename "$f")" ] || docker cp "$CONTAINER:$APPS/$f" raw/
done
for t in cc1 as ld; do
  cmp -s "raw/$t.opt.wasm" "$E/wasm/$t.opt.wasm" || { echo "raw/$t.opt.wasm is not the toolchain's"; exit 1; }
done

# The toolchain's instrumented builds, reproduced, then without Asyncify.
"$WO" --epoch-injection --asyncify --pass-arg=asyncify-removelist@@raw/asyncify_ignore.txt \
  --strip-debug -Os raw/cc1.wasm -o cc1.default.wasm
"$WO" --epoch-injection --strip-debug "$CC1_OPT" raw/cc1.wasm -o cc1.noasync.wasm
for t in as ld; do
  "$OPT" --static "raw/$t.wasm" -o "$t.default.wasm"
  "$OPT" --static --without-asyncify "raw/$t.wasm" -o "$t.noasync.wasm"
done
for t in cc1 as ld; do
  cmp -s "$t.default.wasm" "raw/$t.opt.wasm" || { echo "$t: the recipe does not reproduce $t.opt.wasm"; exit 1; }
  "$LIND_BOOT" --precompile "$t.noasync.wasm" >/dev/null
  echo "$t: $(stat -c %s "$E/root/usr/local/bin/$t") -> $(stat -c %s "$t.noasync.cwasm") bytes of cwasm"
done

# root-na: hard links to root, but its own cc1, as, ld and linker scripts
# (which name the tree's libraries by absolute path).
[ -d "$RNA" ] || cp -al "$E/root" "$RNA"
for t in cc1 as ld; do
  rm -f "$RNA/usr/local/bin/$t"
  cp "$t.noasync.cwasm" "$RNA/usr/local/bin/$t"
done
for f in libc.so libm.so; do
  p=$RNA/usr/lib/x86_64-linux-gnu/$f
  sed "s#$E/root/#$RNA/#g" "$E/root/usr/lib/x86_64-linux-gnu/$f" >"$p.new"
  rm -f "$p" && mv "$p.new" "$p"
done
echo "staged $RNA"
