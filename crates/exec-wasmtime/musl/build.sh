#!/bin/sh
# Build musl's libc.a with every syscall going through ENARX_SYSCALL (see
# enarx_syscall.h), for enarx-exec-wasmtime: in an SGX keep its syscalls
# then reach the shim without an exception, and so without leaving the
# enclave. The version must be the one Rust bundles for
# x86_64-unknown-linux-musl (1.2.5 for the pinned nightly).
#
# Usage: musl/build.sh <musl-1.2.5.tar.gz> [outdir]
#   (from https://musl.libc.org/releases/; outdir defaults to musl/lib, where
#   build.rs looks unless ENARX_MUSL_DIRECT_LIB says otherwise)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tarball=$1
out=${2:-$here/lib}
sha256=a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4

echo "$sha256  $tarball" | sha256sum -c --quiet -
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$tarball" -C "$work"
src=$work/musl-1.2.5

cp "$here/enarx_syscall.h" "$src/arch/x86_64/"
cp "$here/enarx_sysentry.c" "$src/src/internal/"

# The inline syscalls of every C file.
sed -i '1i #include "enarx_syscall.h"' "$src/arch/x86_64/syscall_arch.h"
sed -i 's/"syscall"/"ENARX_SYSCALL"/' "$src/arch/x86_64/syscall_arch.h"
test "$(grep -c '"ENARX_SYSCALL"' "$src/arch/x86_64/syscall_arch.h")" = 7

# The assembly files with a syscall instruction (.S to get the header).
# __set_thread_area is left alone: enarx-exec-wasmtime replaces it.
for f in src/thread/x86_64/syscall_cp src/thread/x86_64/clone \
	src/thread/x86_64/__unmapself src/process/x86_64/vfork \
	src/signal/x86_64/restore; do
	{
		echo '#include "enarx_syscall.h"'
		sed 's/^\([[:space:]]*\)syscall\b/\1ENARX_SYSCALL/' "$src/$f.s"
	} >"$src/$f.S"
	rm "$src/$f.s"
	grep -q ENARX_SYSCALL "$src/$f.S"
done
if grep -rlw syscall "$src/src" --include='*.s' | grep x86_64 | grep -v __set_thread_area; then
	echo "musl: a syscall instruction was left out" >&2
	exit 1
fi

cd "$src"
CFLAGS="-fPIC -O2 -ffunction-sections -fdata-sections" ./configure \
	--disable-shared --disable-wrapper >"$work/configure.log" 2>&1 || {
	cat "$work/configure.log" >&2
	exit 1
}
make -j"$(nproc)" lib/libc.a >"$work/make.log" 2>&1 || {
	tail -30 "$work/make.log" >&2
	exit 1
}
mkdir -p "$out"
cp lib/libc.a "$out/libc.a"
echo "musl: $out/libc.a"
