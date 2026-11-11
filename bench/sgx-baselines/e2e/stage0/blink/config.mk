SYSROOT = /home/lind/lind-wasm/lind-wasm-apps/build/sysroot_merged
LLVM = /home/lind/lind-wasm/clang+llvm-18.1.8-x86_64-linux-gnu-ubuntu-18.04/bin
CC = $(LLVM)/clang --target=wasm32-unknown-wasi --sysroot=$(SYSROOT) -pthread
AR = $(LLVM)/llvm-ar
MODE ?=
TMPDIR = /home/lind/blink-wasm/o/tmp
PREFIX = /usr/local
CFLAGS = -fno-common -O2 -g -fwasm-exceptions -mllvm -wasm-enable-sjlj
CPPFLAGS = -I$(SYSROOT)/include -I$(SYSROOT)/include/wasm32-wasi -DNOLINEAR -D_FILE_OFFSET_BITS=64 -D_DEFAULT_SOURCE -D_BSD_SOURCE -D_GNU_SOURCE
UOPFLAGS = -O2
TARGET_ARCH =
LDFLAGS = -Wl,--import-memory,--export-memory,--max-memory=4294967296,--export=__stack_pointer,--export=__stack_low,--export=__tls_base -L$(SYSROOT)/lib/wasm32-wasi -L$(SYSROOT)/usr/lib/wasm32-wasi
LDLIBS = /home/lind/blink-wasm/o/lind/lind-compat.o -lm
HOST_OS = GNU/Linux
HOST_ARCH = wasm32
HOST_SYSTEM = Linux
CONFIG_HOSTNAME = 19c7c16413f6
CONFIG_COMMAND = ./configure --disable-jit --disable-threads --disable-sockets
CONFIG_ARGUMENTS = -DCONFIG_ARGUMENTS="\"--disable-jit --disable-threads --disable-sockets\""
BLINK_UNAME_V = -DBLINK_UNAME_V="\"NOJIT NOSOCK\""
ZLIB = o/$(MODE)/third_party/libz/zlib.a
