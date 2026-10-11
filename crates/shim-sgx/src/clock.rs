// SPDX-License-Identifier: Apache-2.0

//! Clocks read from the TSC inside the enclave.
//!
//! The host is asked for the time once (see `Handler::clock_init`); after
//! that, `clock_gettime()` extrapolates from the TSC without leaving the
//! enclave. The host could always lie about the time, so this trusts it no
//! more than forwarding every call does.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static READY: AtomicBool = AtomicBool::new(false);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static BASE_TSC: AtomicU64 = AtomicU64::new(0);
static BASE_MONOTONIC_NS: AtomicU64 = AtomicU64::new(0);
static BASE_REALTIME_NS: AtomicU64 = AtomicU64::new(0);

/// Read the time stamp counter.
#[inline]
pub fn rdtsc() -> u64 {
    // Safety: RDTSC is allowed in SGX2 enclaves and has no side effects.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Whether [`init`] has run.
#[inline]
pub fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

/// Anchor the clocks: at TSC value `tsc`, the monotonic clock read
/// `monotonic_ns` and the real-time clock `realtime_ns`.
pub fn init(tsc_hz: u64, tsc: u64, monotonic_ns: u64, realtime_ns: u64) {
    TSC_HZ.store(tsc_hz, Ordering::Relaxed);
    BASE_TSC.store(tsc, Ordering::Relaxed);
    BASE_MONOTONIC_NS.store(monotonic_ns, Ordering::Relaxed);
    BASE_REALTIME_NS.store(realtime_ns, Ordering::Relaxed);
    READY.store(true, Ordering::Release);
}

/// Nanoseconds of the monotonic (`realtime == false`) or real-time clock.
/// Only valid once [`ready`] returns true.
#[inline]
pub fn now_ns(realtime: bool) -> u64 {
    let elapsed = rdtsc().saturating_sub(BASE_TSC.load(Ordering::Relaxed)) as u128;
    let elapsed_ns = (elapsed * 1_000_000_000 / TSC_HZ.load(Ordering::Relaxed) as u128) as u64;
    let base = if realtime {
        &BASE_REALTIME_NS
    } else {
        &BASE_MONOTONIC_NS
    };
    base.load(Ordering::Relaxed) + elapsed_ns
}
