# SGX baselines: TriSeal vs. Gramine vs. Occlum

Runs the same C sources on TriSeal, Gramine and Occlum, all on SGX hardware,
plus native and `gramine-direct` (no SGX) as reference points.

```
src/          workloads; identical C source for every system
gramine/      Makefile + one manifest template shared by all workloads
occlum/       run-docker.sh (dev container) + build.sh (one Occlum instance)
triseal/      build.sh: lind toolchain -> cwasm + Enarx.<app>.toml
run.sh        wall-clock every system on one workload
run-micro.sh  in-process microbenchmarks on every system
```

## Requirements

Checked on lind-server-3 (Xeon Gold 5416S, SGX2 + FLC + EDMM, ~2 GB EPC,
Ubuntu 24.04, kernel 6.11):

- Gramine 1.8 from the Gramine apt repo, and a signing key
  (`gramine-sgx-gen-private-key`, once).
- Docker with the `occlum/occlum:latest-ubuntu20.04` image.
- TriSeal: an `enarx` release build (`$ENARX`, default
  `~/enarx/target/release/enarx`) and a lind-wasm checkout (`$LIND`, default
  `~/lind-wasm`) that has `lindfs/grates/imfs-grate.cwasm`.

## Build and run

```sh
(cd gramine && make APP=forkexecv)           # also builds the native binary
triseal/build.sh forkexecv
occlum/run-docker.sh ./build.sh forkexecv_spawn
./run.sh forkexecv 5
```

Each system can also be run on its own: `make APP=forkexecv run` (or
`direct`) in `gramine/`, the `enarx run` command printed by
`triseal/build.sh`, and `occlum run /bin/<app>` inside `occlum/inst` in the
container.

## Microbenchmarks

`src/microbench.c` times getpid, clock_gettime, pipes, file reads and
writes, and fork/spawn/exec in loops inside one process, so enclave startup
is excluded. It prints ns per operation.

```sh
(cd gramine && make APP=microbench)
STATIC=1 triseal/build.sh microbench
occlum/run-docker.sh ./build.sh microbench
./run-micro.sh 100000 20        # syscall iterations, process iterations
```

Results go to `results/<timestamp>/`: `micro.csv`, one log per system, and
`env.txt` (versions, CPU governor, enclave settings). File tests use an
in-memory filesystem everywhere (Gramine `tmpfs`, Occlum `ramfs`, TriSeal
imfs, `/dev/shm` natively) and wrap around after 1 MB.

Known TriSeal problems that the harness works around:

- `pipe()` returns ENOSYS. musl turns both `pipe()` and `pipe2(fd, 0)` into
  `SYS_pipe`, which the sallyport guest handler does not forward (it only
  handles `SYS_pipe2`). The pipe tests are reported as skipped.
- Exited cages are not reclaimed. Exec fails with ENOMEM after about 11
  fork+exec calls (about 34 forks without exec) in one keep, and the keep
  then hangs instead of exiting. `run-micro.sh` gives each process test its
  own keep and kills any run after `RUN_TIMEOUT` seconds. Use a small
  process iteration count (5) for TriSeal.

### TriSeal file I/O breakdown

`triseal/fileio-analysis.sh` reruns the file tests with `ENARX_SGX_STATS=1`
for the deployed imfs grate, a release build of it, and no grate, and sweeps
the file size with `MB_BLOCKS`. Findings on lind-server-3 (2026-10-10):

- The imfs path never leaves the enclave (0 EEXITs per operation); all of
  its cost is compute inside the keep.
- `lindfs/grates/imfs-grate.cwasm` is a debug build of the Rust imfs grate.
  A release build is 12-25x faster (`file_lseek` 14.9 -> 0.61 us,
  `file_write_64` 19.3 -> 1.6 us, `file_write_4k` 97.9 -> 5.4 us).
- imfs stores files as a linked list of 1 KB chunks and walks it from the
  head on every read and write, so cost grows with the offset: release
  `file_write_4k` takes 1.8 / 5.4 / 16.5 us at 64 KB / 1 MB / 4 MB files.
- About 0.6 us per call is fixed grate dispatch (`file_lseek`: 3i lookup,
  worker lease, Wasm call, fdtables, the imfs lock). Read and write also
  allocate a buffer and copy twice (cage -> grate buffer -> chunk).
- Without a grate, each file call is one ocall but costs two enclave exits
  (~12 us): rawposix issues a `syscall` instruction, which traps as #UD
  before the shim forwards it through sallyport.

### File I/O: TriSeal vs. Gramine

`file-compare.sh` runs the file tests (plus open/close and stat) on native,
Gramine's tmpfs, allowed (plain host), encrypted and trusted files, TriSeal
with the imfs grate (`IMFS_GRATE`, default the optimized
`imfs-chunk-index` build) and TriSeal straight to host files. It reports ns
per call and host syscalls per call, counted with `strace -f -c` for every
system, and runs `src/handoff.c` to check whether a file written by one
process can be read by an exec'd child, as cc1 -> as -> ld needs.

Findings on lind-server-3 (2026-10-10, results/filecmp-20261010-013547):

- Gramine's tmpfs is private to each process: the exec'd child cannot see
  the parent's file. A multi-process compile on Gramine therefore needs
  host-backed files (trusted inputs, encrypted or allowed intermediates),
  which leave the enclave. TriSeal's imfs is shared by all cages and never
  leaves the enclave (0 host syscalls for every file call).
- Against those host-backed files TriSeal is faster: open+close 4.8 us vs
  30.7 us (trusted) / 72 us (encrypted); 4 KB read 1.1 us vs 74 us
  (trusted) / 38 us (encrypted); 4 KB write 1.2 us vs 78 us (encrypted).
- Gramine is faster wherever it stays inside the enclave: tmpfs reads and
  writes (0.1-0.2 us vs ~1.1 us), stat and lseek (0.1-0.3 us vs 3.6 / 0.6 us,
  served from its in-enclave caches), and 64-byte encrypted writes and reads
  (buffered, 0.14-0.16 us).
- When TriSeal does go to the host, each call costs ~12 us (two enclave
  exits) against ~4.2 us for Gramine's allowed files (one).

## How the systems differ

- **fork.** Gramine creates a new enclave for every child, so fork/exec cost
  grows with `sgx.enclave_size`. Occlum has no `fork()`, so workloads that
  fork get a `*_spawn.c` variant using `posix_spawn`, which `run.sh` picks
  automatically. TriSeal runs children as cages inside one enclave.
- **Files.** In TriSeal, `execv()` only sees files that are preloaded into
  the imfs grate, so `triseal/build.sh` writes a `PRELOADS` entry for every
  binary the workload runs. Gramine checks `trusted_files` hashes, and
  Occlum's image is integrity-protected.
- **Enclave size.** TriSeal reserves 64 GB (`ENCL_SIZE_BITS = 36`) and
  commits pages on demand. Gramine defaults to `1G` without EDMM, which can
  be changed with `ENCLAVE_SIZE=` and `EDMM=1`. Occlum builds SGX1-only
  enclaves with `USER_SPACE=600MB`. Match these settings before comparing
  results, and report how sensitive the numbers are to them.
- **Counters.** `ENARX_SGX_STATS=1` prints TriSeal's EENTER/EEXIT/AEX/ocall
  counts in the same format as Gramine's `sgx.enable_stats`. Gramine's
  apt package is a release build, which rejects `sgx.enable_stats` no matter
  what `sgx.debug` says, so counters need Gramine built from source with
  `--buildtype=debugoptimized`. Occlum has no built-in equivalent.

## First numbers

`forkexecv`, wall-clock ms, 3 runs, lind-server-3, 2026-09-26. The CPU
governor was `powersave` with turbo on, and the TriSeal build still printed
debug `[instrument]` lines, so these numbers only give orders of magnitude.

| system                    | ms          |
|---------------------------|-------------|
| native                    | 4-6         |
| gramine-direct            | 85-87       |
| gramine-sgx, 256M / 1G / 4G | 640 / 1720 / 12400 |
| occlum-sgx (spawn), 600MB | 1466-1472   |
| triseal-sgx (imfs grate)  | 7231-9084   |

All of these are dominated by enclave startup. Loops inside the workload are
needed to measure the cost of each operation.
