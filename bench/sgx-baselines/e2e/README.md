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

## Fork/exec optimizations

lind-wasm branch `triseal-forkexec-opt` (local only; worktree
`~/sgx-bench/e2e/lw-opt` on lind-server-3, based on `enarx-7-13`) plus the
imfs grate branch `triseal-forkexec-opt` (logical clock for file times),
with TriSeal `open-futex` unchanged (64 GB enclave, ENCL_SIZE_BITS 36):

| commit | change |
|---|---|
| f4a09e50c | zero anonymous MAP_FIXED mappings under SGX (cc1 -O2 fix) |
| 17ef115a4 | compile out [instrument] tracing unless LIND_INSTRUMENT is set |
| bae4eba96 | cache deserialized modules across exec (fstat-keyed LRU) |
| 69ce71bd4 | port lind-wasm#1385 (no grate worker pools for non-grates) |
| e6624a22a | drop the grate flag and old pool when a cage execs a non-grate |
| a0e55a6a7 | zero only memory that can hold stale data; fix SGX munmap |
| c2385ae13 | more instrument markers (spawn child, exec entry, exit, syscalls) |

Lua build (results/e2e-opt-20261010-120634): 292 s -> **124 s**
(123.7 / 123.0 / 125.2), with in-toto 294 s -> 125 s; Gramine is 138 s.
The binaries are unchanged (same hashes). Exited cages are now reclaimed,
so the 1 TB enclave is no longer needed. One fork+exec with a cached
module is ~0.31 s (was ~0.83 s); `forkexec-profile.sh` shows where it goes.
`src/cachetest.c` checks that rewriting an exec'd file is not hidden by
the module cache; `abtest.sh` and `run-opt.sh` are the timing drivers.

What is left per process is mostly SGX page management: every new cage
commits fresh EPC pages on first touch (EAUG + EACCEPT) and every exit
trims them again (the shim's munmap). Reusing committed pages across cages
would need changes to the shim's ledger (mmledger) or a memory pool in
the wasmtime/lind layer.

## No ocalls after startup

TriSeal branch `triseal-zero-ocall` (local, on `open-futex` 0df4e331)
moves everything a running workload asks of the host into the enclave:
page commits (the heap is mapped on the host once and committed pages are
zeroed and reused instead of trimmed), futex waits (spinning), new threads
(4 threads wait in the enclave), `clock_gettime` (TSC) and console output
(buffered until exit). The host also stops re-running a handler's sallyport
block after a nested page fault.

Lua build: no ocall from the start of the compile until the console is
flushed at exit (133,276 before). Cold 47.3 s, first build in a running
keep 35.9 s, warm 32.0 s (124.4 / 114.4 / 110.9 s before; Gramine from
source 127.2 / 128.2 / 127.3 s). The price is CPU time: waiting threads
spin, about 3.9 cores busy during the build. Details and data:
`zero-ocall-20261010/` in the data repository.

`zero-ocall.sh` counts the ocalls of a build against a probe for a runtime
built with `triseal/stats-overlay.py` (the `ENARX_SGX_STATS` module plus an
enarxcall histogram and a snapshot at the first console write).
`lua-triseal.sh` takes `NODUMP=1` so that dumping lua and luac does not
count.

## Preloads in shared memory, attested ("preloading signing")

`lua-triseal-shm.sh` runs the build with the inputs packed on the host into
one bundle of two archives (`mkpreload <bundle> <pipeline...> --section
<sources...>`) and an in-toto grate above the IMFS grate:

1. Before lind starts, the keep has the host map the bundle and a dump file
   outside the enclave (`[shm]` in Enarx.toml).
2. IMFS copies the compiler pipeline out of that memory and stages it, then
   stops; the in-toto grate reads the archive from IMFS's memory, hashes
   every file and signs an in-toto statement (DSSE, ECDSA P-256 key made in
   the enclave).
3. IMFS resumes with the sources, and stops again for the second
   attestation, which records the hash of the first envelope.
4. make runs. At exit lua, luac and the attestations go into one archive,
   copied to the dump memory and committed with one ocall.

`verify-attest.py` checks the envelopes on the host against the bundle
(signature with openssl, subjects = the files and their SHA-256, chain).
`TAMPER=1` flips a byte of the bundle after packing: IMFS rejects that
archive and nothing runs.

`shm-ocalls.sh` counts what this asks of the host, with a runtime built
with `triseal/stats-overlay.py`. Once IMFS runs, staging the bundle (345 MB),
both attestations and the compile take no ocall; the dump takes one
(`CommitShm`), followed by the console output (2 writes) and the exit.
Everything else (about 390 ocalls) is keep startup: the attestation of the
keep, reading the two grate modules, the thread pool and mapping the two
files. The build takes 53.4 s (53.0-53.9; 47.3 s with host preloads and no
attestation). Code: TriSeal `triseal-shm-preload`, lind-wasm
`triseal-shm-preload`, lind-wasm-example-grates `imfs-preload-attest`.

## Known TriSeal issues found here

- Anonymous `MAP_FIXED`, `brk` shrink-then-grow and munmap left stale data,
  and munmap returned ENOTSUP under SGX (fixed on triseal-forkexec-opt).
- `pipe()` and stdout redirection do not work under the imfs grate (fds
  0-2 always go to the host).
- Exited cages were never reclaimed: every forked child got a grate worker
  pool whose Stores kept its memory alive (fixed on triseal-forkexec-opt).
- When a cage thread panics (for example on that ENOMEM), the keep hangs
  instead of exiting.
- The keep's exit status does not reflect the workload's: a failed make
  still exits 0.
- lind does not flush stdio buffers when a cage exits.
