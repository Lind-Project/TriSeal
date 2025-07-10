// SPDX-License-Identifier: Apache-2.0

//! I/O functionality for keeps

pub mod null;

use wasi_common::WasiFile;
use wasmtime_wasi::preview1::types::Rights;

pub fn stdio_file(mut file: impl WasiFile + 'static) -> (Box<dyn WasiFile>, Rights) {
    // Ensure wasmtime can detect the TTY.
    let mode = if file.isatty() {
        Rights::all().difference(Rights::FD_SEEK | Rights::FD_TELL)
    } else {
        Rights::all()
    };
    (Box::new(file), mode)
}
