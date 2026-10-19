// SPDX-License-Identifier: Apache-2.0

//! The `[shm]` regions: host files mapped outside the keep before lind
//! starts.
//!
//! The preload bundle (packed on the host ahead of time with `mkpreload`) and
//! the file the dump archive goes to are opened here, mapped by the host
//! (`SYS_MAPSHM`), and registered with rawposix. From then on a grate copies
//! its preloads out of, and its dump into, that memory without leaving the
//! keep. Committing a dump asks nothing of the host either: the host cuts
//! the dump file to the length of the archive in it when the keep exits.

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
        // Kept open: the host cuts it to the dump's length at exit.
        let fd = file.into_raw_fd();
        let addr =
            map(fd, capacity, true).with_context(|| format!("cannot map the dump file {path}"))?;
        // The host cuts the file to the archive's length at exit.
        let commit: rawposix::shm::DumpCommit = Box::new(|_len| Ok(()));
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

/// Pass a phase mark to the shim. Through libc, so that with the musl from
/// `musl/build.sh` it does not leave the enclave either.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn phase(n: u64) {
    use sallyport::item::enarxcall::SYS_PHASE;
    unsafe { libc::syscall(SYS_PHASE as libc::c_long, n) };
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn map(_fd: i32, _len: usize, _writable: bool) -> Result<usize> {
    bail!("[shm] regions need an SGX keep")
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn phase(_n: u64) {}
