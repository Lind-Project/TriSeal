// SPDX-License-Identifier: Apache-2.0

//! Enarx call item definitions

pub mod sev;
pub mod sgx;

use core::mem::size_of;

/// `get_attestation` syscall number used by the shim.
///
/// See <https://github.com/enarx/enarx/issues/966>
#[allow(dead_code)]
pub const SYS_GETATT: i64 = 0xEA01;

/// `get_key` syscall number used by the shim.
///
/// See <https://github.com/enarx/enarx/issues/2110>
#[allow(dead_code)]
pub const SYS_GETKEY: i64 = 0xEA02;

/// `map_shm(fd, len, writable)` syscall number used by the shim: ask the host
/// to map an open host file outside the enclave, returning its address. A
/// writable mapping is first sized to `len`.
#[allow(dead_code)]
pub const SYS_MAPSHM: i64 = 0xEA03;

/// `commit_shm(fd, len)` syscall number used by the shim: ask the host to
/// truncate a file mapped with `map_shm` to `len` bytes and flush it.
#[allow(dead_code)]
pub const SYS_COMMITSHM: i64 = 0xEA04;

/// `direct_init()` syscall number used by the shim: set up the memory the
/// keep shares with the host for its console and phase marks, and return
/// the address of the shim's direct syscall entry (0 if there is none).
#[allow(dead_code)]
pub const SYS_DIRECT_INIT: i64 = 0xEA05;

/// Number the shim's own `syscall` instruction carries when a syscall that
/// came in through the direct entry needs the host: the handler entered for
/// it runs the request the direct path left in the sallyport block.
#[allow(dead_code)]
pub const SYS_SALLY: i64 = 0xEA06;

/// `phase(n)` syscall number used by the shim: record that the workload
/// entered phase `n` (1 = the build starts), for the host to count from.
#[allow(dead_code)]
pub const SYS_PHASE: i64 = 0xEA07;

/// Upper half of the value the shim leaves the enclave with (in `r8`, by
/// EEXIT at CSSA 0) after an `exit_group()` taken through its direct syscall
/// entry, the lower half being the status: the host then exits the process,
/// as it does for an `exit_group` request.
#[allow(dead_code)]
pub const EXIT_GROUP_MARK: u64 = 0xEA1D_0000_0000_0000;

/// Kinds of memory [`Number::MapHostShared`] maps.
pub mod shared {
    /// Console output, as records the host writes out when the keep exits.
    pub const CONSOLE: usize = 0;
    /// The phase page: the phase mark and the shim's counters.
    pub const PHASE: usize = 1;
}

/// Payload of an [`Item`](super::Item) of [`Kind::Enarxcall`](super::Kind::Enarxcall).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(8))]
pub struct Payload {
    pub num: Number,
    pub argv: [usize; 4],
    pub ret: usize,
}

pub(crate) const USIZE_COUNT: usize = size_of::<Payload>() / size_of::<usize>();

impl From<&mut [usize; USIZE_COUNT]> for &mut Payload {
    #[inline]
    fn from(buf: &mut [usize; USIZE_COUNT]) -> Self {
        debug_assert_eq!(size_of::<Payload>(), USIZE_COUNT * size_of::<usize>());
        unsafe { &mut *(buf as *mut _ as *mut _) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
/// Number of an [`Item`](super::Item) of [`Kind::Enarxcall`](super::Kind::Enarxcall).
pub enum Number {
    /// MemInfo request call number.
    MemInfo = 0x00,

    /// Memory ballooning request call number.
    BalloonMemory = 0x01,

    /// Cpuid instruction call number.
    Cpuid = 0x02,

    /// SGX quote request call number.
    GetSgxQuote = 0x03,

    /// [SGX `TargetInfo`](sgx::TargetInfo) request call number.
    GetSgxTargetInfo = 0x04,

    /// SGX quote size request call number.
    GetSgxQuoteSize = 0x05,

    /// SNP VCEK request call number.
    GetSnpVcek = 0x06,

    /// Notify the host about `mmap()`.
    MmapHost = 0x07,

    /// Notify the host about `mprotect()`.
    MprotectHost = 0x08,

    /// Notify the host about `munmap()`.
    MunmapHost = 0x09,

    /// Trim SGX pages call number.
    SgxModifyPageType = 0x10,

    /// Park the current thread
    Park = 0x11,

    /// UnPark all parked threads
    UnPark = 0x12,

    /// Spawn a new thread
    Spawn = 0x13,

    /// Register a new sallyport block
    NewSallyport = 0x14,

    /// Map an open host file outside the enclave
    MapShm = 0x15,

    /// Truncate and flush a file mapped with `MapShm`
    CommitShm = 0x16,

    /// Map anonymous host memory shared with the host (see [`shared`])
    MapHostShared = 0x17,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_size() {
        assert_eq!(size_of::<Payload>(), USIZE_COUNT * size_of::<usize>())
    }

    #[test]
    fn tech_assignments() {
        assert_ne!(sev::TECH, sgx::TECH);
    }
}
