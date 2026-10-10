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

## Results

lind-server-3, 2026-10-10 (results/e2e-20261010-045438), wall-clock seconds
for the whole build, 3 runs each (governor powersave, turbo on):

| configuration | mean | runs | vs native |
|---|---|---|---|
| native | 5.79 | 5.78 / 5.78 / 5.82 | 1x |
| TriSeal (imfs, SGX) | 291.8 | 291.4 / 291.8 / 292.4 | 50x |
| TriSeal + in-toto | 294.0 | 294.1 / 293.8 | 51x |
| Gramine, encrypted files, 1G | 138.1 | 138.0 / 138.1 / 138.3 | 24x |
| Gramine, allowed files, 1G | 136.4 | 136.1 / 136.5 / 136.5 | 24x |
| Gramine, encrypted, EDMM | 152.3 | 152.2 / 152.4 / 152.3 | 26x |

- Every run produced a working `lua`. TriSeal's is byte-identical (after
  strip) to the native build. The in-toto run signs a link with 61 source
  materials whose `lua`/`luac` product hashes match the dumped binaries.
- TriSeal's keep startup plus preloading the 205 files is 28.1 s
  (`PROBE=1`). The rest is dominated by process creation: one
  fork+exec costs ~0.75 s for `as` (18 MB module) and ~3.5 s for `cc1`
  (248 MB), because every exec reads the whole module out of IMFS through
  3i and deserializes it again. The build execs cc1 33 times (~115 s).
  Caching the deserialized module across execs is the obvious next step.
- File I/O is not the bottleneck on either system for this build: Gramine
  with encrypted files is within 2 s of plain files.

## Known TriSeal issues found here

- Anonymous `MAP_FIXED` and `brk` shrink-then-grow return stale data
  (fixed locally as above).
- Exited cages are never reclaimed (worked around with a larger enclave).
- When a cage thread panics (for example on that ENOMEM), the keep hangs
  instead of exiting.
- The keep's exit status does not reflect the workload's: a failed make
  still exits 0.
- lind does not flush stdio buffers when a cage exits.
