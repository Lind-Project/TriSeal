// SPDX-License-Identifier: Apache-2.0

//! Link the musl built by `musl/build.sh`, whose syscalls reach the SGX
//! shim without an exception, if there is one: in `ENARX_MUSL_DIRECT_LIB`,
//! or in `musl/lib` next to this file. Without it the payload keeps the
//! musl Rust bundles, and every syscall traps into the shim.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(enarx_direct_musl)");
    println!("cargo:rerun-if-env-changed=ENARX_MUSL_DIRECT_LIB");

    if std::env::var("TARGET").as_deref() != Ok("x86_64-unknown-linux-musl") {
        return;
    }
    let dir = match std::env::var_os("ENARX_MUSL_DIRECT_LIB") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("musl/lib"),
    };
    let lib = dir.join("libc.a");
    println!("cargo:rerun-if-changed={}", lib.display());
    if lib.is_file() {
        // Searched before the musl Rust bundles.
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-cfg=enarx_direct_musl");
    }
}
