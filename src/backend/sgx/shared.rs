// SPDX-License-Identifier: Apache-2.0

//! Memory the keep shares with the host outside the enclave, and what the
//! host does with it when the keep exits.
//!
//! The keep writes into this memory without leaving the enclave:
//!
//! - the console: stdout and stderr, as `[fd: u8][len: u32][bytes]`
//!   records after a `u64` count of the bytes used, written out here at exit;
//! - the phase page: the workload's phase mark and the shim's counters (see
//!   [`phase`]);
//! - the dump file mapped with `MapShm`: the keep copies its dump archive
//!   into it, and the file is cut here to the archive's length at exit, so
//!   that committing a dump needs no call to the host.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sallyport::item::enarxcall::shared;

/// Size of the console memory.
pub const CONSOLE_SIZE: usize = 64 << 20;

/// Size of the phase page.
pub const PHASE_SIZE: usize = 4096;

static CONSOLE: AtomicUsize = AtomicUsize::new(0);
static PHASE: AtomicUsize = AtomicUsize::new(0);
static DUMP: Mutex<Option<(libc::c_int, usize, usize)>> = Mutex::new(None);

/// Layout of the phase page, in `u64` words.
pub mod phase {
    /// Set by the host when it wants each phase mark acknowledged.
    pub const WATCH: usize = 0;
    /// The current phase, set by the keep.
    pub const PHASE: usize = 1;
    /// The phase the host has acknowledged.
    pub const ACK: usize = 2;
    /// Shim counters when the phase was last set.
    pub const AT_PHASE: usize = 8;
    /// Shim counters when the keep exited.
    pub const AT_EXIT: usize = 24;
    /// Number of counters in each set.
    pub const COUNTERS: usize = 12;
    /// Counters from here on are only kept by a shim built to count the
    /// pages in use (feature `in-use`); they are printed if not zero.
    pub const WORKING_SET: usize = 8;
    /// Names of the counters.
    pub const NAMES: [&str; COUNTERS] = [
        "syscalls through the direct entry",
        "syscalls through #UD",
        "page faults handled by the shim (#PF)",
        "pages committed",
        "pages committed ahead of use",
        "cpuid through #UD",
        "  of the pages committed ahead, by mprotect()",
        "  of the pages committed ahead, by madvise(MADV_WILLNEED)",
        "pages in use (lower bound: touched)",
        "  peak of pages in use (lower bound)",
        "pages in use (upper bound: with reused committed pages)",
        "  peak of pages in use (upper bound)",
    ];
}

/// Map `len` bytes of anonymous memory for `kind` and remember it.
pub fn map(kind: usize, len: usize) -> Result<usize, i32> {
    let (slot, size) = match kind {
        shared::CONSOLE => (&CONSOLE, CONSOLE_SIZE),
        shared::PHASE => (&PHASE, PHASE_SIZE),
        _ => return Err(libc::EINVAL),
    };
    if len != size {
        return Err(libc::EINVAL);
    }
    let addr = slot.load(Ordering::Acquire);
    if addr != 0 {
        return Ok(addr);
    }
    let addr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if addr == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::ENOMEM));
    }
    slot.store(addr as usize, Ordering::Release);
    Ok(addr as usize)
}

/// Remember the dump file mapped at `addr`.
pub fn set_dump(fd: libc::c_int, addr: usize, len: usize) {
    *DUMP.lock().unwrap() = Some((fd, addr, len));
}

/// The phase page, if the keep has mapped it.
pub fn phase_page() -> Option<&'static [std::sync::atomic::AtomicU64]> {
    let addr = PHASE.load(Ordering::Acquire);
    if addr == 0 {
        return None;
    }
    // Safety: the page stays mapped for the life of the process and is
    // only accessed atomically.
    Some(unsafe {
        std::slice::from_raw_parts(addr as *const std::sync::atomic::AtomicU64, PHASE_SIZE / 8)
    })
}

/// Write out the console and cut the dump file. Called once, when the keep
/// exits.
pub fn finish() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        write_console();
        cut_dump();
    });
}

fn write_console() {
    let addr = CONSOLE.load(Ordering::Acquire);
    if addr == 0 {
        return;
    }
    // Safety: the memory stays mapped; the keep no longer writes to it.
    let mem = unsafe { std::slice::from_raw_parts(addr as *const u8, CONSOLE_SIZE) };
    let used = (u64::from_le_bytes(mem[..8].try_into().unwrap()) as usize).min(CONSOLE_SIZE - 8);
    let records = &mem[8..8 + used];
    let mut at = 0;
    while at + 5 <= records.len() {
        let fd = records[at] as libc::c_int;
        let len = u32::from_le_bytes(records[at + 1..at + 5].try_into().unwrap()) as usize;
        let end = (at + 5 + len).min(records.len());
        if fd == libc::STDOUT_FILENO || fd == libc::STDERR_FILENO {
            write_all(fd, &records[at + 5..end]);
        }
        at = end;
    }
}

fn write_all(fd: libc::c_int, mut data: &[u8]) {
    while !data.is_empty() {
        let n = unsafe { libc::write(fd, data.as_ptr() as *const _, data.len()) };
        if n <= 0 {
            return;
        }
        data = &data[n as usize..];
    }
}

/// The dump region holds a preload archive (magic, version, entries, payload
/// length, digest, then the payload) if the keep dumped anything: cut the
/// file to its length, or to nothing if there is no archive.
fn cut_dump() {
    let Some((fd, addr, cap)) = *DUMP.lock().unwrap() else {
        return;
    };
    const HEADER: usize = 56;
    // Safety: the file stays mapped until the process exits.
    let mem = unsafe { std::slice::from_raw_parts(addr as *const u8, cap) };
    let len = if cap >= HEADER && &mem[..8] == b"LINDPRLD" {
        let payload = u64::from_le_bytes(mem[16..24].try_into().unwrap()) as usize;
        payload.saturating_add(HEADER).min(cap)
    } else {
        0
    };
    unsafe { libc::ftruncate(fd, len as libc::off_t) };
}
