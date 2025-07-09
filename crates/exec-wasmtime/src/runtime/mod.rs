// SPDX-License-Identifier: Apache-2.0

//! The Enarx Wasm runtime and all related functionality

mod identity;
mod io;
//mod net;

use self::io::null::Null;
// use self::io::stdio_file;
//use self::net::{connect_file, listen_file};

use super::{Package, Workload};

use wasmtime_lind_common::LindCommonCtx;
use wasmtime_lind_multi_process::{LindCtx, LindHost, CAGE_START_ID, THREAD_START_ID};
use wasmtime_lind_utils::lind_syscall_numbers::EXIT_SYSCALL;
use wasmtime_lind_utils::LindCageManager;
use rawposix::safeposix::dispatcher::lind_syscall_api;
use std::sync::Arc;
use cap_std::fs::Dir;
use wasmtime_wasi_threads::WasiThreadsCtx;
use std::sync::atomic::AtomicU64;
use anyhow::{anyhow, Result, bail};
use wasmtime::{
    Func, InstantiateType, StoreLimits, ValType,
};

use anyhow::Context;
use enarx_config::{Config, File};
// use wasi_common::snapshots::preview_1::types::Rights;
// use wasi_common::WasiFile;
use wasmtime::{AsContextMut, Engine, Linker, Module, Store, Val};
// use wasmtime_wasi::{Stderr, Stdin, Stdout, StdoutStream, StdinStream, DirPerms, FilePerms};
// use wasmtime_wasi::WasiCtxBuilder;
use wasi_common::sync::WasiCtxBuilder;
// use std::fs;
// use wasmtime_wasi::preview1::{add_to_linker_sync, WasiP1Ctx};
// use wasmtime_wasi::{add_to_linker, WasiCtxBuilder};
// use wiggle::tracing::{instrument, trace_span};
use wiggle::tracing::trace_span;

// Declare the context 
#[derive(Default, Clone)]
struct MyCtx {
    preview1_ctx: Option<wasi_common::WasiCtx>,
    wasi_threads: Option<Arc<WasiThreadsCtx<MyCtx>>>,
    lind_common_ctx: Option<LindCommonCtx>,
    lind_fork_ctx: Option<LindCtx<MyCtx, Option<enarx_config::Config>>>,
}

impl AsMut<wasi_common::WasiCtx> for MyCtx {
    fn as_mut(&mut self) -> &mut wasi_common::WasiCtx {
        self.preview1_ctx
            .as_mut()
            .expect("preview1_ctx must be initialized before use")
    }
}

impl MyCtx {
    pub fn fork(&self) -> Self {
        // we want to do a real fork for wasi_preview1 context since glibc uses the environment variable
        // related interface here
        let forked_preview1_ctx = match &self.preview1_ctx {
            Some(ctx) => Some(ctx.fork()),
            None => None
        };

        // and we also want to fork the lind-common context and lind-multi-process context
        let forked_lind_fork_ctx = match &self.lind_fork_ctx {
            Some(ctx) => Some(ctx.fork()),
            None => None
        };

        let forked_lind_common_ctx = match &self.lind_common_ctx {
            Some(ctx) => Some(ctx.fork()),
            None => None
        };

        // besides preview1_ctx, lind_common_ctx and forked_lind_fork_ctx, we do not
        // care about other context since they are not used by glibc so we can just share
        // them between processes
        let forked_host = Self {
            preview1_ctx: forked_preview1_ctx,
            lind_fork_ctx: forked_lind_fork_ctx,
            lind_common_ctx: forked_lind_common_ctx,
            wasi_threads: self.wasi_threads.clone(),
        };

        return forked_host;
    }
}

impl LindHost<MyCtx, Option<enarx_config::Config>> for MyCtx {
    fn get_ctx(&self) -> LindCtx<MyCtx, Option<enarx_config::Config>> {
        self.lind_fork_ctx.clone().unwrap()
    }
}

// The Enarx Wasm runtime
#[derive(Clone)]
pub struct Runtime;

impl Runtime {
    // Execute an Enarx [Package]
    // #[instrument]
    pub fn execute(package: Package) -> anyhow::Result<Vec<Val>> {
        let (prvkey, crtreq) =
            identity::generate().context("failed to generate a private key and CSR")?;

        let Workload { webasm, config } = package.try_into()?;
        let enarx_conf = config;
        let Config {
            steward,
            args,
            files,
            env,
        } = enarx_conf.clone().unwrap_or_default();

        let certs = if let Some(url) = steward {
            // Obtaining attestation certificates 
            identity::steward(&url, crtreq).context("failed to attest to Steward")?
        } else {
            // Generating a self-signed certificate
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
        // config.async_support(false);

        let engine = trace_span!("initialize Wasmtime engine")
            .in_scope(|| Engine::new(&config))
            .context("failed to create execution engine")?;

        // let wasi = builder.build();
        let Host = MyCtx::default();

        // let mut wstore = trace_span!("initialize Wasmtime store")
        //     .in_scope(|| Store::new(&engine, builder.build_p1()));
        let mut wstore = trace_span!("initialize Wasmtime store")
            .in_scope(|| Store::new(&engine, Host));

        let module = trace_span!("compile Wasm")
            .in_scope(|| Module::from_binary(&engine, &webasm))
            .context("failed to compile Wasm module")?;

        let lind_manager = Arc::new(LindCageManager::new(0));
        rawposix::safeposix::dispatcher::lindrustinit(0);
        lind_manager.increment();

        // Set up the WASI. In lind-wasm, we predefine all the features we need are `thread` and `wasipreview1`
        // so we manually add them to the linker without checking the input
        let mut linker = trace_span!("setup linker").in_scope(|| Linker::new(&engine));
        // Setup WASI-p1
        trace_span!("link WASI")
            .in_scope(|| wasi_common::sync::add_to_linker(&mut linker, |s: &mut MyCtx| AsMut::<wasi_common::WasiCtx>::as_mut(s)))
            .context("failed to setup linker and link WASI")?;
        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();
        builder.inherit_stdin();
        builder.inherit_stderr();
        // builder.preopened_dir("/home", ".", DirPerms::all(), FilePerms::all()).expect("failed to open current directory");
        let dir = Dir::open_ambient_dir("/home", cap_std::ambient_authority()).expect("failed to open /home");
        builder.preopened_dir(dir, ".").expect("failed to open current directory");
        wstore.data_mut().preview1_ctx = Some(builder.build());

        // Setup WASI-thread
        trace_span!("link WASI-thread")
            .in_scope(|| wasmtime_wasi_threads::add_to_linker(&mut linker, &wstore, &module, |s: &mut MyCtx| s.wasi_threads.as_ref().unwrap()))
            .context("failed to setup linker and link WASI")?;

        // attach Lind-Common-Context to the host
        let shared_next_cageid = Arc::new(AtomicU64::new(1));
        {
            wasmtime_lind_common::add_to_linker::<MyCtx, Option<enarx_config::Config>>(&mut linker, |host| {
                host.lind_common_ctx.as_ref().unwrap()
            })?;
            wstore.data_mut().lind_common_ctx = Some(LindCommonCtx::new(shared_next_cageid.clone())?);
        }

        // attach Lind-Multi-Process-Context to the host
        {
            wstore.data_mut().lind_fork_ctx = Some(LindCtx::new(
                module.clone(),
                linker.clone(),
                lind_manager.clone(),
                // self.clone(),
                webasm.clone(),
                enarx_conf.clone(),
                shared_next_cageid.clone(),
                |host| host.lind_fork_ctx.as_mut().unwrap(),
                |host| host.fork(),
                |webasm, enarx_conf, path, args, pid, next_cageid, lind_manager, envs| {
                    Runtime::execute_with_lind(
                        webasm.clone(),
                        enarx_conf.clone(),
                        lind_manager.clone(),
                        pid as u64,
                        next_cageid.clone(),
                    )
                },
            )?);
        }

        wstore.data_mut().wasi_threads = Some(Arc::new(WasiThreadsCtx::new(
            module.clone(),
            Arc::new(linker.clone()),
        )?));

        let result = wasmtime_wasi::runtime::with_ambient_tokio_runtime(|| {
            Runtime::load_main_module(
                &mut wstore,
                &mut linker,
                &module,
                CAGE_START_ID as u64,
                &args,
            )
            .with_context(|| {
                format!(
                    "failed to run main module"
                )
            })
        });

        match result {
            Ok(ref res) => {
                let mut code = 0;
                let retval: &Val = res.get(0).unwrap();
                if let Val::I32(res) = retval {
                    code = *res;
                }
                // exit the thread
                if rawposix::interface::lind_thread_exit(
                    CAGE_START_ID as u64,
                    THREAD_START_ID as u64,
                ) {
                    // we clean the cage only if this is the last thread in the cage
                    // exit the cage with the exit code
                    lind_syscall_api(1, EXIT_SYSCALL as u32, 0, code as u64, 0, 0, 0, 0, 0);

                    // main cage exits
                    lind_manager.decrement();
                }
            }
            Err(e) => {
                // Exit the process if Wasmtime understands the error;
                // otherwise, fall back on Rust's default error printing/return
                // code.
                return Err(wasi_common::maybe_exit_on_error(e));
            }
        }

        result
    }

    pub fn execute_with_lind(
        // Wasm module
        webasm: Vec<u8>,
        // Enarx keep configuration
        config: Option<Config>,
        lind_manager: Arc<LindCageManager>,
        pid: u64,
        next_cageid: Arc<AtomicU64>,
    ) -> Result<Vec<Val>> {

        println!("Executing with Lind: pid: {}", pid);

        let enarx_conf = config;
        let Config {
            steward,
            args,
            files,
            env,
        } = enarx_conf.clone().unwrap_or_default();

        let mut config = wasmtime::Config::new();
        // config.wasm_reference_types(true);       // Enable reference types
        // config.wasm_function_references(true);   // Enable function references
        // config.wasm_gc(true);                    // Enable garbage collection
        config.wasm_backtrace(true);
        config.native_unwind_info(true);                // Enable threads
        config.debug_info(true);                        // Enable debug info
        config.memory_init_cow(false);
        // config.async_support(false);

        let engine = trace_span!("initialize Wasmtime engine")
            .in_scope(|| Engine::new(&config))
            .context("failed to create execution engine")?;

        // let wasi = builder.build();
        let Host = MyCtx::default();

        // let mut wstore = trace_span!("initialize Wasmtime store")
        //     .in_scope(|| Store::new(&engine, builder.build_p1()));
        let mut wstore = trace_span!("initialize Wasmtime store")
            .in_scope(|| Store::new(&engine, Host));

        let module = trace_span!("compile Wasm")
            .in_scope(|| Module::from_binary(&engine, &webasm))
            .context("failed to compile Wasm module")?;

        // Set up the WASI. In lind-wasm, we predefine all the features we need are `thread` and `wasipreview1`
        // so we manually add them to the linker without checking the input
        let mut linker = trace_span!("setup linker").in_scope(|| Linker::new(&engine));
        // Setup WASI-p1
        trace_span!("link WASI")
            .in_scope(|| wasi_common::sync::add_to_linker(&mut linker, |s: &mut MyCtx| AsMut::<wasi_common::WasiCtx>::as_mut(s)))
            .context("failed to setup linker and link WASI")?;
        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();
        builder.inherit_stdin();
        builder.inherit_stderr();
        // builder.preopened_dir("/home", ".", DirPerms::all(), FilePerms::all()).expect("failed to open current directory");
        let dir = Dir::open_ambient_dir("/home", cap_std::ambient_authority()).expect("failed to open /home");
        builder.preopened_dir(dir, ".").expect("failed to open current directory");
        wstore.data_mut().preview1_ctx = Some(builder.build());

        // Setup WASI-thread
        trace_span!("link WASI-thread")
            .in_scope(|| wasmtime_wasi_threads::add_to_linker(&mut linker, &wstore, &module, |s: &mut MyCtx| s.wasi_threads.as_ref().unwrap()))
            .context("failed to setup linker and link WASI")?;

        // attach Lind-Common-Context to the host
        let shared_next_cageid = Arc::new(AtomicU64::new(1));
        {
            wasmtime_lind_common::add_to_linker::<MyCtx, Option<enarx_config::Config>>(&mut linker, |host| {
                host.lind_common_ctx.as_ref().unwrap()
            })?;
            // Create a new lind ctx with the next cage ID since we are going to fork
            wstore.data_mut().lind_common_ctx = Some(LindCommonCtx::new_with_pid(
                pid as i32,
                next_cageid.clone(),
            )?);
        }

        // attach Lind-Multi-Process-Context to the host
        {
            wstore.data_mut().lind_fork_ctx = Some(LindCtx::new_with_pid(
                module.clone(),
                linker.clone(),
                lind_manager.clone(),
                // self.clone(),
                webasm.clone(),
                enarx_conf.clone(),
                pid as i32,
                next_cageid.clone(),
                |host| host.lind_fork_ctx.as_mut().unwrap(),
                |host| host.fork(),
                |webasm, enarx_conf, path, args, pid, next_cageid, lind_manager, envs| {
                    Runtime::execute_with_lind(
                        webasm.clone(),
                        enarx_conf.clone(),
                        lind_manager.clone(),
                        pid as u64,
                        next_cageid.clone(),
                    )
                },
            )?);
        }

        wstore.data_mut().wasi_threads = Some(Arc::new(WasiThreadsCtx::new(
            module.clone(),
            Arc::new(linker.clone()),
        )?));

        let result = wasmtime_wasi::runtime::with_ambient_tokio_runtime(|| {
            Runtime::load_main_module(
                &mut wstore,
                &mut linker,
                &module,
                pid as u64,
                &args,
            )
            .with_context(|| {
                format!(
                    "failed to run main module"
                )
            })
        });

        result
    }

    fn load_main_module(
        store: &mut Store<MyCtx>,
        linker: &mut Linker<MyCtx>,
        module: &Module,
        pid: u64,
        args: &[String],
    ) -> Result<Vec<Val>> {
        let instance = linker
            .instantiate_with_lind(
                &mut *store,
                &module,
                InstantiateType::InstantiateFirst(pid),
            )
            .context(format!(
                "failed to instantiate"
            ))?;

        // If `_initialize` is present, meaning a reactor, then invoke
        // the function.
        if let Some(func) = instance.get_func(&mut *store, "_initialize") {
            func.typed::<(), ()>(&store)?.call(&mut *store, ())?;
        }

        // Look for the specific function provided or otherwise look for
        // "" or "_start" exports to run as a "main" function.
        let func = instance
            .get_func(&mut *store, "")
            .or_else(|| instance.get_func(&mut *store, "_start"));

        let stack_low = instance.get_stack_low(store.as_context_mut()).unwrap();
        let stack_pointer = instance.get_stack_pointer(store.as_context_mut()).unwrap();
        store.as_context_mut().set_stack_base(stack_pointer as u64);
        store.as_context_mut().set_stack_top(stack_low as u64);

        // retrieve the epoch global
        let lind_epoch = instance
            .get_export(&mut *store, "epoch")
            .and_then(|export| export.into_global())
            .expect("Failed to find epoch global export!");

        // retrieve the handler (underlying pointer) for the epoch global
        let pointer = lind_epoch.get_handler(&mut *store);

        // initialize the signal for the main thread of the cage
        rawposix::interface::lind_signal_init(
            pid,
            pointer as *mut u64,
            THREAD_START_ID,
            true, /* this is the main thread */
        );

        // see comments at signal_may_trigger for more details
        rawposix::interface::signal_may_trigger(pid);

        let result = match func {
            Some(func) => Runtime::invoke_func(store, func, &args),
            None => Ok(vec![]),
        };
        result
    }

    fn invoke_func(store: &mut Store<MyCtx>, func: Func, args: &[String]) -> Result<Vec<Val>> {
        let ty = func.ty(&store);
        if ty.params().len() > 0 {
            eprintln!(
                "warning: using `--invoke` with a function that takes arguments \
                 is experimental and may break in the future"
            );
        }
        // let mut args = self.module_and_args.iter().skip(1);
        let mut args = args.iter();
        let mut values = Vec::new();
        for ty in ty.params() {
            let val_str = args
                .next()
                .ok_or_else(|| anyhow!("not enough arguments for invoke"))?;
            let val = match ty {
                ValType::I32 => Val::I32(val_str.parse()?),
                ValType::I64 => Val::I64(val_str.parse()?),
                ValType::F32 => Val::F32(val_str.parse::<f32>()?.to_bits()),
                ValType::F64 => Val::F64(val_str.parse::<f64>()?.to_bits()),
                _ => bail!("unsupported argument type {:?}", ty),
            };
            values.push(val);
        }

        // Invoke the function and then afterwards print all the results that came
        // out, if there are any.
        let mut results = vec![Val::null_func_ref(); ty.results().len()];
        func.call(&mut *store, &values, &mut results)
            .with_context(|| { format!("failed to invoke command default") });

        Ok(results)
    }

    
}
