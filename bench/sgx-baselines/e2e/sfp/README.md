# Single-file programs: TriSeal vs. Gramine vs. Occlum

gcc 15.2 builds two of Stephen McCamant's large single-file C programs
(https://people.csail.mit.edu/smcc/projects/single-file-programs/) on
native Linux, TriSeal, Gramine and Occlum, with the same make, compiler,
flags and Makefile everywhere:

| program  | source                          | what is built               |
|----------|---------------------------------|-----------------------------|
| oggenc.c | Ogg Vorbis encoder, 58K lines   | `oggenc`, linked with `-lm` |
| gcc.c    | gcc's cc1, 754K lines (21.9 MB) | `gcc.o`, compiled only (`-c`) |

The sources are not edited. What it takes to build 2004-2006 C with gcc 15
and glibc 2.35 (the staged sysroot) is in `sfp.conf`:

- `-std=gnu89 -fpermissive -w`: build them as the C they were written in
  (implicit declarations and int, K&R definitions, `bool` not a keyword),
  with those errors made warnings again, and no warnings printed.
- `-idirafter compat`: an empty `compat/sys/sysctl.h`. gcc.c includes it
  (glibc dropped it in 2.32) but on Linux uses nothing from it.
- gcc.c is compiled, not linked: it references `sys_nerr`, which glibc no
  longer offers to new programs.
- gzip.c is left out: it defines its own `static strchr`, which conflicts
  with glibc's declaration, and no flag gets it past that without editing
  the source.

`stage.sh` downloads the sources, records their SHA-256 in `SHA256SUMS`
(the page publishes none) and lays out one tree per program with a
one-rule Makefile. `check.sh` smoke-tests the output on the host: oggenc
must run (`--version`); gcc.o must be an x86-64 relocatable object
defining `main`.

## Drivers

| script           | what it runs                                                    |
|------------------|-----------------------------------------------------------------|
| `native.sh`      | the build on the host                                           |
| `triseal.sh`     | in a TriSeal keep under the IMFS grate, preloads from host files (`PRELOADS`), output written back to the host at exit |
| `triseal-shm.sh` | as `lua-triseal-shm.sh`: preloads from shared memory as one bundle of two archives, each attested by the in-toto grate, output in the dump |
| `gramine.sh`     | in Gramine 1.8 (`MODE=encrypted` or `allowed` for the sources and outputs; `ENCLAVE_SIZE`, 4G for gcc.c) |
| `occlum.sh`      | in Occlum (container `occlum/occlum:latest-ubuntu20.04`), with make, gcc and cc1 built by `occlum/sfp/build-tools.sh` (PIE; make uses vfork, Occlum has no fork) |

Each takes `PROBE=1` (run `make --version` with the same setup: the TEE's
startup alone); `triseal.sh`, `gramine.sh` and `native.sh` take `TWICE=1`
(two builds in one TEE instance: the first, and the warm one).

- `coldwarm.sh`: cold, startup, first and warm builds, `N` runs each.
- `after-start.sh`: what each build asks of the host after it starts.
  TriSeal: counters of the stats runtime at the phase mark the IMFS grate
  sets when it starts make, and at exit. Gramine: `sgx.enable_stats`,
  build minus probe. All of them, Occlum included: host syscalls of the
  whole TEE process tree (`strace -f -c`), build minus probe; Occlum has
  no counters of its own, so this is its ocall measure.
- `run-all.sh`: both, into one directory, with the versions used.
