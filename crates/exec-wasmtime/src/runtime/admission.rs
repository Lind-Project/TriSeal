// SPDX-License-Identifier: Apache-2.0

//! Which modules lind may load: those whose SHA-256 is in the list compiled
//! into this runtime (`cwasm-allowlist.txt`, or the file named by
//! `ENARX_CWASM_ALLOWLIST` when it was built, see `build.rs`).
//!
//! The host supplies every module the keep runs: the workload, the grates it
//! execs from host paths, the toolchain in the preload bundle. A precompiled
//! module is native code that runs with all the keep's secrets in reach, so
//! a forged one could sign anything with the keep's keys; recording what ran
//! would not help, since that code could rewrite the record. So a module is
//! checked before lind loads it, against a list that is part of the keep's
//! measurement, and one that is not in the list ends the run.

use sha2::{Digest, Sha256};
use std::fmt::Write;

include!(concat!(env!("OUT_DIR"), "/cwasm_allowlist.rs"));

/// What the keep exits with when it refuses a module (126, as a shell does
/// for a command it cannot execute).
const REFUSED_EXIT: i32 = 126;

/// Have lind check every module against the list from now on. Call before
/// lind loads the first module.
pub fn register() {
    wasmtime_lind_utils::admission::register(Box::new(|name, bytes, precompiled| {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if ALLOWED.iter().any(|(allowed, _)| *allowed == digest) {
            return Ok(());
        }
        let mut hex = String::with_capacity(64);
        for byte in digest {
            write!(hex, "{byte:02x}").unwrap();
        }
        let kind = if precompiled {
            "precompiled module"
        } else {
            "module"
        };
        // Not an error lind would hand to the program that asked for the
        // module: a substituted module is an attack, and the run ends here.
        eprintln!("[exec-wasmtime] refused {kind} {name} (sha256 {hex}): not in the allowlist");
        std::process::exit(REFUSED_EXIT);
    }));
}

#[cfg(test)]
mod tests {
    use super::ALLOWED;

    #[test]
    fn allowlist_compiled_in() {
        assert!(!ALLOWED.is_empty());
        for (i, (digest, name)) in ALLOWED.iter().enumerate() {
            assert!(!name.is_empty(), "entry {i} has no name");
            assert!(
                ALLOWED[..i].iter().all(|(other, _)| other != digest),
                "{name} is listed twice"
            );
        }
    }
}
