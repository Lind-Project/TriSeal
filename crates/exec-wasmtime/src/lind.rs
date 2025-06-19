use anyhow::{Result, Context};
use lind_wasm::cli::RunCommand;
use clap::Parser;
use std::fs::File;
use std::io::Write;
use tempfile::NamedTempFile;

pub fn execute_wasm(wasm_bytes: &[u8], args: &[String]) -> Result<i32> {
    
    let mut file = NamedTempFile::new()
        .context("failed to create temporary wasm file")?;
    file.write_all(wasm_bytes)
        .context("failed to write wasm to temporary file")?;
    let path = file.path().to_string_lossy().to_string();

    let mut argv = vec!["wasmtime run --wasi threads=y --wasi preview2=n".to_string(), path];
    argv.extend(args.iter().cloned());

    let cmd = RunCommand::parse_from(argv);
    cmd.execute().context("lind-wasm execution failed")?;

    Ok(0) 
}
