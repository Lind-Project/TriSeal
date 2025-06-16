// SPDX-License-Identifier: Apache-2.0

//! The Enarx Wasm runtime and all related functionality

mod identity;
mod io;
//mod net;

use self::io::null::Null;
use self::io::stdio_file;
//use self::net::{connect_file, listen_file};

use super::{Package, Workload};

use anyhow::Context;
use enarx_config::{Config, File};
use wasi_common::snapshots::preview_1::types::Rights;
use wasi_common::WasiFile;
use wasmtime::{AsContextMut, Engine, Linker, Module, Store, Val};
use wasmtime_wasi::{Stderr, Stdin, Stdout, StdoutStream, StdinStream, DirPerms, FilePerms};
use wasmtime_wasi::WasiCtxBuilder;
use std::fs;
use wasmtime_wasi::preview1::{add_to_linker_sync, WasiP1Ctx};
use wiggle::tracing::{instrument, trace_span};

// The Enarx Wasm runtime
pub struct Runtime;

impl Runtime {
    // Execute an Enarx [Package]
    #[instrument]
    pub fn execute(package: Package) -> anyhow::Result<Vec<Val>> {
        let (prvkey, crtreq) =
            identity::generate().context("failed to generate a private key and CSR")?;

        let Workload { webasm, config } = package.try_into()?;
        let Config {
            steward,
            args,
            files,
            env,
        } = config.unwrap_or_default();

        let certs = if let Some(url) = steward {
            identity::steward(&url, crtreq).context("failed to attest to Steward")?
        } else {
            identity::selfsigned(&prvkey).context("failed to generate self-signed certificates")?
        }
        .into_iter()
        .map(rustls::Certificate)
        .collect::<Vec<_>>();

        let mut config = wasmtime::Config::new();
        // config.wasm_reference_types(true);       // Enable reference types
        // config.wasm_function_references(true);   // Enable function references
        // config.wasm_gc(true);                    // Enable garbage collection
        config.wasm_backtrace(true);
        config.native_unwind_info(true);                // Enable threads
        config.debug_info(true);                        // Enable debug info
        config.memory_init_cow(false);
        config.async_support(false);

        let engine = trace_span!("initialize Wasmtime engine")
            .in_scope(|| Engine::new(&config))
            .context("failed to create execution engine")?;

        let mut linker = trace_span!("setup linker").in_scope(|| Linker::new(&engine));
        trace_span!("link WASI")
            .in_scope(|| add_to_linker_sync(&mut linker, |s| s))
            .context("failed to setup linker and link WASI")?;

        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();
        builder.inherit_stdin();
        builder.inherit_stderr();
        // builder.args(&["main.wasm", "/etc/resolv.conf", "/etc/resolv.conf"]);
        builder.preopened_dir("/home", ".", DirPerms::all(), FilePerms::all()).expect("failed to open current directory");
       // builder.preopened_dir("/tmp", "/tmp", DirPerms::all(), FilePerms::all()).expect("failed to open current directory");

        //fs::File::open("/home/lind/enarx/test.txt").map_err(|err| format!("error opening input fuckup /home/lind/enarx/test.txt: {}", err)).unwrap();

        let mut wstore = trace_span!("initialize Wasmtime store")
            .in_scope(|| Store::new(&engine, builder.build_p1()));

        let module = trace_span!("compile Wasm")
            .in_scope(|| Module::from_binary(&engine, &webasm))
            .context("failed to compile Wasm module")?;
        trace_span!("link Wasm")
            .in_scope(|| linker.module(&mut wstore, "", &module))
            .context("failed to link module")?;

        let mut ctx = wstore.as_context_mut();
        let ctx = ctx.data_mut();

        let func = trace_span!("get default function")
            .in_scope(|| linker.get_default(&mut wstore, ""))
            .context("failed to get default function")?;

        let mut values = vec![Val::null_any_ref(); func.ty(&wstore).results().len()];
        trace_span!("execute default function")
            .in_scope(|| func.call(wstore, Default::default(), &mut values))
            .context("failed to execute default function")?;
        Ok(values)
    }
}
