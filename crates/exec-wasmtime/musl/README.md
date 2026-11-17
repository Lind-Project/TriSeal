# musl with syscalls through the SGX shim's direct entry

In an SGX keep the payload's `syscall` instruction is illegal: it raises #UD,
the host enters the enclave again to run the shim's handler, and the handler
returns through the host once more. `build.sh` builds musl's `libc.a` with
every `syscall` instruction replaced by `ENARX_SYSCALL` (`enarx_syscall.h`):
once the payload has set `__enarx_sysentry` to the shim's direct entry
(`crates/shim-sgx/src/direct.rs`), a syscall jumps there and comes back
without leaving the enclave. Until then, and outside an SGX keep, it is an
ordinary `syscall`.

```bash
curl -LO https://musl.libc.org/releases/musl-1.2.5.tar.gz   # the version Rust bundles
crates/exec-wasmtime/musl/build.sh musl-1.2.5.tar.gz         # -> crates/exec-wasmtime/musl/lib/libc.a
cargo build --release
```

`build.rs` links `musl/lib/libc.a` (or the directory in
`ENARX_MUSL_DIRECT_LIB`) ahead of the musl Rust bundles and sets
`cfg(enarx_direct_musl)`, under which `enarx-exec-wasmtime` installs the
entry at startup. Without it the keep works as before, every syscall
trapping into the shim. rustix is built with `--cfg rustix_use_libc`
(`.cargo/config.toml`) so that wasmtime's own syscalls go through musl too.

The files patched are those of Gramine's musl patch, which does the same
for Gramine's LibOS, except `__set_thread_area`, which
`enarx-exec-wasmtime` replaces.
