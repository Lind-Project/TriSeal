# enarx-exec-wasmtime

`enarx-exec-wasmtime` - the Enarx WebAssembly loader

`enarx-exec-wasmtime` is responsible for loading and running WebAssembly modules
inside an Enarx keep.

Users generally won't execute `enarx-exec-wasmtime` directly, but for test/debugging
purposes it can be used to run a .wasm file with given command-line
arguments and environment variables.

### Example invocation

```console
$ wat2wasm ../tests/wasm/return_1.wat
$ RUST_LOG=enarx-exec-wasmtime=info RUST_BACKTRACE=1 cargo run -- return_1.wasm
    Finished dev [unoptimized + debuginfo] target(s) in 0.03s
     Running `target/x86_64-unknown-linux-musl/debug/enarx-exec-wasmtime return_1.wasm`
[INFO  enarx-exec-wasmtime] version 0.2.0 starting up
[WARN  enarx-exec-wasmtime] 🌭DEV-ONLY BUILD, NOT FOR PRODUCTION USE🌭
[INFO  enarx-exec-wasmtime] opts: RunOptions {
        envs: [],
        module: Some(
            "return_1.wasm",
        ),
        args: [],
    }
[INFO  enarx-exec-wasmtime] reading module from "return_1.wasm"
[INFO  enarx-exec-wasmtime] running workload
[WARN  enarx-exec-wasmtime::workload] inheriting stdio from calling process
[INFO  enarx-exec-wasmtime] got result: Ok(
        [
            I32(
                1,
            ),
        ],
    )
```

If no filename is given, `enarx-exec-wasmtime` expects to read the WebAssembly module
from file descriptor 3, so this would be equivalent:
```console
$ RUST_LOG=enarx-exec-wasmtime=info RUST_BACKTRACE=1 cargo run -- 3< return_1.wasm
 ```

### Module allowlist

lind loads only the modules (`.cwasm` or `.wasm`) whose SHA-256 is in
`cwasm-allowlist.txt`, which is compiled in and so is part of the keep's
measurement. Any other module, whether it is the workload, a grate or a tool
a build execs, ends the run with status 126. To run other modules, list them
(`sha256sum *.cwasm` output) in a file and build with
`ENARX_CWASM_ALLOWLIST=<file>`; the keep then has another measurement.
`lind-boot --precompile` reproduces a `.cwasm` from its `.wasm` byte for byte,
so anyone can check what an entry is.


License: Apache-2.0

### in-toto attestation of a run

Every workload run produces two in-toto attestations: `run-start` right before the
lind-wasm runtime (`lind_boot::execute_wasmtime`) starts and `run-end` right after it
returns. Each is an in-toto Statement v1 inside a DSSE v1 envelope, signed with the keep
identity key (P-256 on SGX/KVM), the key whose hash is bound into the TEE attestation
report. See `crates/enarx-config/Enarx_toml.md` (`intoto`) for the configuration and the
predicate contents.

```toml
[intoto]
dir = "/var/lib/enarx/attestations"   # unset: printed to stderr
```

Files written per run `<id>`: `lind-wasm-run-start.<id>.intoto.json`,
`lind-wasm-run-end.<id>.intoto.json` and `lind-wasm-signer.<id>.pub.json` (public key in
in-toto/securesystemslib format). To verify, base64-decode `payload`, rebuild the DSSE
PAE `DSSEv1 <len> application/vnd.in-toto+json <len> <payload>` and check `sig`
(base64, ASN.1 ECDSA-P256-SHA256) against the public key; `run-end` carries the sha256 of
the `run-start` file so the two can be tied together.

When the workload's first cage records evidence of its own (the in-toto grate), `run-end` is
signed when that cage hands in its key and the hash of the last link of its chain (lind's
`LIND_EVIDENCE`, see `src/runtime/attest.rs`), names both, and goes back to the workload
with `run-start`, the identity key and the attestation report (on SGX, enarx's DER evidence:
the quote, whose report data is the SHA-256 of the identity key's SPKI, and Intel's CRLs and
TCB info), so that the recorder's whole chain is anchored to the quote.
