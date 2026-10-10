# End-to-end compile: TriSeal vs. Gramine

Builds Lua 5.5.1 with its own Makefile (`make all`, 33 `.c` files, `ar`,
`ranlib`, two links) with gcc 15.2.0 in three settings:

- **native**: gcc 15.2.0 built natively from the same source as TriSeal's.
- **TriSeal** (`lua-triseal.sh`): make, gcc, cc1, as, ld, ar and ranlib are
  lind cages under the imfs grate in one SGX keep. Sources, headers, objects
  and gcc's temporary files stay in enclave memory; lua and luac are dumped
  to the host at exit. `INTOTO=1` wraps make in `in-toto-cli run` and signs
  a link over the sources and the two binaries.
- **Gramine** (`lua-gramine.sh`): the native gcc and the host's make and
  binutils, one enclave per process. Gramine's tmpfs is private to each
  process, so the work tree and `TMPDIR` are a host mount, either encrypted
  (`MODE=encrypted`, sources encrypted up front) or plain (`MODE=allowed`).

All three use the same headers, crt files and glibc 2.35 (staged under
`root/`) and the same flags: `-std=gnu99 -O2 -Wall -Wextra -DLUA_USE_POSIX`.

## Setup

1. `e2e/stage.sh` copies the lind toolchain from the `lind-sep25` container
   (lind-wasm-apps with PR 288, lind-wasm with #1397), precompiles it for
   the host's lind-boot, stages headers and libraries, and builds the native
   gcc.
2. in-toto-cli (lind-wasm-apps PR 284) must be built against the sysroot
   the enarx runtime was built from, without the `debug::` imports of a
   container build. `compile_in-toto.sh` runs with a private
   `LIND_WASM_ROOT` whose sysroot, lind-boot and binaryen link to
   `~/lind-wasm`, and with a copy of the Rust toolchain (`rustup toolchain
   link lind-sgx-bench`) so that `make_lind_libc.sh --patch-std` does not
   touch `nightly-2026-02-11` itself.
3. enarx is built from TriSeal's `open-futex` branch (0df4e331: getdents64,
   which `in-toto-cli verify` needs, plus the pipe, dup and chdir fixes)
   against a private copy of `~/lind-wasm/src` with two local changes:
   - `rawposix` `mmap_inner`: the SGX shortcut for anonymous `MAP_FIXED`
     mappings returned the old bytes. It now zeroes the range, since glibc
     relies on fresh memory being zero after `brk` shrink-and-regrow. Without
     this, cc1 aborts at `-O2` with `free(): invalid pointer`
     (`src/zerotest.c` shows it).
   - `shim-sgx` `ENCL_SIZE_BITS` 36 -> 40. Exited cages keep their ~4 GB of
     linear memory, so a 64 GB enclave runs out after about 12 cages and the
     keep hangs; the Lua build needs about 106.

## Known TriSeal issues found here

- Anonymous `MAP_FIXED` and `brk` shrink-then-grow return stale data
  (fixed locally as above).
- Exited cages are never reclaimed (worked around with a larger enclave).
- When a cage thread panics (for example on that ENOMEM), the keep hangs
  instead of exiting.
- The keep's exit status does not reflect the workload's: a failed make
  still exits 0.
- lind does not flush stdio buffers when a cage exits.
