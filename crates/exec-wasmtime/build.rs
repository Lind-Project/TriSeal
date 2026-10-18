// SPDX-License-Identifier: Apache-2.0

//! Link the musl built by `musl/build.sh`, whose syscalls reach the SGX
//! shim without an exception, if there is one: in `ENARX_MUSL_DIRECT_LIB`,
//! or in `musl/lib` next to this file. Without it the payload keeps the
//! musl Rust bundles, and every syscall traps into the shim.
//!
//! Compile in the list of modules lind may load (`cwasm-allowlist.txt`, or
//! the file in `ENARX_CWASM_ALLOWLIST`), see `src/runtime/admission.rs`.

use std::fmt::Write;
use std::path::PathBuf;

fn main() {
    allowlist();

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

/// Write `$OUT_DIR/cwasm_allowlist.rs`: `ALLOWED`, the SHA-256 and name of
/// each module in the list.
fn allowlist() {
    println!("cargo:rerun-if-env-changed=ENARX_CWASM_ALLOWLIST");
    let path = match std::env::var_os("ENARX_CWASM_ALLOWLIST") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("cwasm-allowlist.txt"),
    };
    println!("cargo:rerun-if-changed={}", path.display());
    let list = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read the module allowlist {}: {e}", path.display()));

    let mut out = String::from("/// SHA-256 and name of each module lind may load.\n");
    out.push_str("const ALLOWED: &[([u8; 32], &str)] = &[\n");
    let mut entries = 0;
    for (n, line) in list.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hex, name) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let name = name.trim().trim_start_matches('*');
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            panic!("{}:{}: not a SHA-256 and a name: {line}", path.display(), n + 1);
        }
        let mut digest = String::new();
        for i in (0..64).step_by(2) {
            let byte = u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
            write!(digest, "{byte:#04x}, ").unwrap();
        }
        writeln!(out, "    ([{digest}], {name:?}),").unwrap();
        entries += 1;
    }
    out.push_str("];\n");
    if entries == 0 {
        println!(
            "cargo:warning=the module allowlist {} is empty: the keep will load no module",
            path.display()
        );
    }
    let dest = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("cwasm_allowlist.rs");
    std::fs::write(dest, out).unwrap();
}
