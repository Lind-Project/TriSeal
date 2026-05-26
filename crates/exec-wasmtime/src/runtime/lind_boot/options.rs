use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct LindBootOptions {
    pub args: Vec<String>,
    pub vars: Vec<(String, Option<String>)>,
    pub preloads: Vec<(String, PathBuf)>,
    pub precompile: bool,
    pub wasmtime_backtrace: bool,
    pub enable_fpcast: bool,
    pub thread_stack_size: usize,
    pub chroot_lindfs: bool,
}

impl LindBootOptions {
    pub fn wasm_file(&self) -> &str {
        &self.args[0]
    }
}
