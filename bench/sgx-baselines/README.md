# SGX baselines: TriSeal vs. Gramine vs. Occlum

Runs the same C sources on TriSeal, Gramine and Occlum, all on SGX hardware,
plus native and `gramine-direct` (no SGX) as reference points.

```
src/       workloads; identical C source for every system
gramine/   Makefile + one manifest template shared by all workloads
occlum/    run-docker.sh (dev container) + build.sh (one Occlum instance)
triseal/   build.sh: lind toolchain -> cwasm + Enarx.<app>.toml
run.sh     wall-clock every system on one workload
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
