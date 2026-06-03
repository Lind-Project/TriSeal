// SPDX-License-Identifier: Apache-2.0

//! The Enarx Wasm runtime and all related functionality

mod identity;
mod io;
//mod net;
// mod lind_boot;

use super::{Package, Workload};

use anyhow::Context;
use enarx_config::{Config, File};
use std::ffi::CString;
use std::path::{Path, PathBuf};

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

        let (prvkey, crtreq) =
            identity::generate().context("failed to generate a private key and CSR")?;
        let _certs = if let Some(url) = config.steward.as_ref() {
            identity::steward(url, crtreq).context("failed to attest to Steward")?
        } else {
            identity::selfsigned(&prvkey).context("failed to generate self-signed certificates")?
        }
        .into_iter()
        .map(rustls::Certificate)
        .collect::<Vec<_>>();

        let mut args = vec!["main.wasm".to_string()];
        args.extend(config.args);

        let vars = config
            .env
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect();

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
            preloads: vec![
                ("env".to_string(), PathBuf::from("/home/alice/lind-wasm/lindfs/lib/libc.cwasm")),
                ("env".to_string(), PathBuf::from("/home/alice/lind-wasm/lindfs/lib/libm.cwasm")),
            ],
            thread_stack_size: 64 * 1024 * 1024,
        };

        // chroot_to_lindfs();

        rawposix_start(0);
        let code = lind_boot::execute_wasmtime(options)?;
        rawposix_shutdown();

        Ok(vec![Val::I32(code)])
    }
}

fn chroot_to_lindfs() {
    unsafe {
        let lindfs_path = CString::new(LINDFS_ROOT).unwrap();

        if !Path::new(LINDFS_ROOT).is_dir() {
            panic!("The configured lindfs does not exist: {}", LINDFS_ROOT);
        }

        let ret = libc::chroot(lindfs_path.as_ptr());
        if ret != 0 {
            panic!(
                "Failed to chroot to {}: {}",
                LINDFS_ROOT,
                std::io::Error::last_os_error()
            );
        }
        let root = CString::new("/").unwrap();
        let ret = libc::chdir(root.as_ptr());
        if ret != 0 {
            panic!(
                "Failed to chdir to / after chroot: {}",
                std::io::Error::last_os_error()
            )
        }
    }
}
