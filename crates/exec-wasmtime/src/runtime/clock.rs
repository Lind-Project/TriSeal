// SPDX-License-Identifier: Apache-2.0

//! The date the run sees (`[clock]` in Enarx.toml).
//!
//! The SGX shim never asks the host for the time of day: its real-time clock
//! counts from the epoch the workload declares here, before anything of the
//! run reads it, and advances with the TSC.

use anyhow::{anyhow, Result};

/// Nanoseconds since the Unix epoch at `epoch` seconds, if the shim can
/// represent them (up to year 2262).
pub fn epoch_ns(epoch: u64) -> Result<u64> {
    epoch
        .checked_mul(1_000_000_000)
        .filter(|ns| *ns <= i64::MAX as u64)
        .ok_or_else(|| anyhow!("[clock] epoch {epoch} is out of range"))
}

/// Have the shim's real-time clock read `epoch` (seconds since the Unix
/// epoch) now. Through libc, like the phase marks, so that with the musl
/// from `musl/build.sh` it does not leave the enclave. A shim that keeps no
/// clock of its own (outside SGX) answers ENOSYS; the run then has the
/// host's clock.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn declare(epoch: u64) -> Result<()> {
    use sallyport::item::enarxcall::SYS_CLOCK_EPOCH;
    let ns = epoch_ns(epoch)?;
    if unsafe { libc::syscall(SYS_CLOCK_EPOCH as libc::c_long, ns) } == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error() {
        e if e.raw_os_error() == Some(libc::ENOSYS) => Ok(()),
        e => Err(anyhow!("the shim refused the [clock] epoch: {e}")),
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn declare(epoch: u64) -> Result<()> {
    epoch_ns(epoch).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::epoch_ns;

    #[test]
    fn epochs_up_to_2262_are_accepted() {
        assert_eq!(epoch_ns(0).unwrap(), 0);
        assert_eq!(epoch_ns(1_760_659_200).unwrap(), 1_760_659_200_000_000_000);
        assert!(epoch_ns(9_223_372_036).is_ok());
        assert!(epoch_ns(9_223_372_037).is_err());
        assert!(epoch_ns(u64::MAX).is_err());
    }
}
