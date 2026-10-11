// SPDX-License-Identifier: Apache-2.0

//! The `[shm]` regions: host files mapped outside the keep before lind
//! starts.
//!
//! The preload bundle (packed on the host ahead of time with `mkpreload`) and
//! the file the dump archive goes to are opened here, mapped by the host
//! (`SYS_MAPSHM`), and registered with rawposix. From then on a grate copies
//! its preloads out of, and its dump into, that memory without leaving the
//! keep; committing the dump (`SYS_COMMITSHM`) is the one host call the
//! regions need while lind runs.

use anyhow::{bail, Context, Result};
use enarx_config::Shm;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, IntoRawFd};

const DEFAULT_DUMP_SIZE: u64 = 256 << 20;

/// Map the configured files and register them with rawposix.
pub fn setup(shm: &Shm) -> Result<()> {
    if let Some(path) = shm.preload.as_deref() {
        let file =
            File::open(path).with_context(|| format!("cannot open the preload bundle {path}"))?;
        let len = file.metadata()?.len() as usize;
        if len == 0 {
            bail!("the preload bundle {path} is empty");
        }
        let addr = map(file.as_raw_fd(), len, false)
            .with_context(|| format!("cannot map the preload bundle {path}"))?;
        // The mapping outlives the file descriptor.
        unsafe { rawposix::shm::register_preload(addr as *const u8, len) };
    }

    if let Some(path) = shm.dump.as_deref() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("cannot create the dump file {path}"))?;
        let capacity = shm.dump_size.unwrap_or(DEFAULT_DUMP_SIZE) as usize;
        // Kept open: the commit truncates it to the dump's length.
        let fd = file.into_raw_fd();
        let addr =
            map(fd, capacity, true).with_context(|| format!("cannot map the dump file {path}"))?;
        let commit: rawposix::shm::DumpCommit = Box::new(move |len| commit(fd, len));
        unsafe { rawposix::shm::register_dump(addr as *mut u8, capacity, commit) };
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn raw_syscall(nr: i64, a: usize, b: usize, c: usize) -> isize {
    let ret: isize;
    unsafe {
        std::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            lateout("rcx") _, // clobbered
            lateout("r11") _, // clobbered
        )
    }
    ret
}

/// Have the host map `fd` outside the keep (`len` bytes, sized first when
/// writable). The shim refuses an address that overlaps the keep.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn map(fd: i32, len: usize, writable: bool) -> Result<usize> {
    use sallyport::item::enarxcall::SYS_MAPSHM;
    match raw_syscall(SYS_MAPSHM, fd as usize, len, writable as usize) {
        ret if ret < 0 => Err(std::io::Error::from_raw_os_error(-ret as i32).into()),
        addr => Ok(addr as usize),
    }
}

/// Have the host truncate the dump file to `len` bytes and flush it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn commit(fd: i32, len: usize) -> Result<(), i32> {
    use sallyport::item::enarxcall::SYS_COMMITSHM;
    match raw_syscall(SYS_COMMITSHM, fd as usize, len, 0) {
        ret if ret < 0 => Err(ret as i32),
        _ => Ok(()),
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn map(_fd: i32, _len: usize, _writable: bool) -> Result<usize> {
    bail!("[shm] regions need an SGX keep")
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn commit(_fd: i32, _len: usize) -> Result<(), i32> {
    Err(-(libc::ENOSYS))
}
