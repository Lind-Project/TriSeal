// SPDX-License-Identifier: Apache-2.0

//! The Enarx Wasm runtime and all related functionality

mod admission;
mod attest;
mod identity;
mod io;
mod shm;
//mod net;
// mod lind_boot;

use super::{Package, Workload};

use anyhow::Context;
use enarx_config::{Config, File};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rawposix::init::{rawposix_shutdown, rawposix_start};
use wasmtime::Val;
use lind_boot;

pub const LINDFS_ROOT: &str = "/home/alice/lind-wasm/lindfs";

// The Enarx Wasm runtime
#[derive(Clone)]
pub struct Runtime;

impl Runtime {
    pub fn execute(package: Package) -> anyhow::Result<Vec<Val>> {
        let workload: Workload = package.try_into()?;
        let config = workload.config.clone().unwrap_or_default();

        let (prvkey, crtreq, report) =
            identity::generate().context("failed to generate a private key and CSR")?;
        let certs = if let Some(url) = config.steward.as_ref() {
            identity::steward(url, crtreq).context("failed to attest to Steward")?
        } else {
            identity::selfsigned(&prvkey).context("failed to generate self-signed certificates")?
        };

        let mut args = vec![attest::SUBJECT_NAME.to_string()];
        args.extend(config.args.iter().cloned());

        let vars = config
            .env
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect();

        // Map the [shm] regions before lind starts, so that the preload
        // bundle and the dump file are there for the first grate.
        if let Some(regions) = config.shm.as_ref() {
            shm::setup(regions).context("failed to set up the [shm] regions")?;
        }

        // Have lind load only the modules in the compiled-in allowlist,
        // from the workload on.
        admission::register();

        // Hand the workload's phase marks (LIND_PHASE) to the shim, which
        // records them where the host can count from them.
        rawposix::phase::register(Box::new(shm::phase));

        let options = lind_boot::cli::CliOptions {
            verbose: 0,
            debug: false,
            precompile: false,
            wasmtime_backtrace: false,
            enable_fpcast: false,
            wasm_bytes: Some(workload.webasm.clone()),

            // args[0] is still guest argv[0], even though module bytes come from wasm_bytes.
            args,

            vars,
            preloads: Vec::new(),
            thread_stack_size: 64 * 1024 * 1024,
            // The [shm] regions are mapped by the host and registered above.
            preload_shm: None,
            dump_shm: None,
            dump_shm_size: 0,
        };

        // -- in-toto: attest what is about to run, signed with the keep identity key --
        let identity_spki = identity::public_key_der(&prvkey)?;
        let keep = attest::KeepInfo {
            technology: identity::technology()?,
            identity_key: &prvkey,
            identity_spki: &identity_spki,
            certificates: &certs,
            steward: config.steward.as_ref(),
            report: &report,
        };
        let attestor = attest::Attestor::new(&config.intoto, keep, &workload.webasm)
            .context("failed to set up in-toto attestation")?
            .map(|attestor| Arc::new(Mutex::new(attestor)));

        if let Some(attestor) = attestor.as_ref() {
            attestor
                .lock()
                .unwrap()
                .start(&attest::Invocation {
                    argv: &options.args,
                    env: &config.env,
                    preloads: &options.preloads,
                })
                .context("failed to emit in-toto run-start attestation")?;

            // The workload's recorder hands in its key and chain (LIND_EVIDENCE)
            // and gets run-end back, signed with the identity key.
            let attestor = attestor.clone();
            rawposix::evidence::register(Box::new(move |request| {
                attestor.lock().unwrap().evidence(request).map_err(|e| {
                    eprintln!("[in-toto] cannot sign run-end for the recorder: {e:#}");
                    -libc::EINVAL
                })
            }));
        }

        rawposix_start(0);
        let result = lind_boot::execute_wasmtime(options);
        if result.is_ok() {
            rawposix_shutdown();
        }

        // -- in-toto: attest how the run ended, even when the runtime failed,
        // unless run-end went to the workload's recorder already --
        let attested = attestor
            .as_ref()
            .map(|attestor| {
                let outcome = match &result {
                    Ok(code) => attest::Outcome::Exited(*code),
                    Err(e) => attest::Outcome::Failed(e),
                };
                attestor
                    .lock()
                    .unwrap()
                    .end(outcome)
                    .context("failed to emit in-toto run-end attestation")
            })
            .transpose();

        let code = result?;
        attested?;

        Ok(vec![Val::I32(code)])
    }
}
