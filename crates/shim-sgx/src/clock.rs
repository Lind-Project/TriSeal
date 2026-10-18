// SPDX-License-Identifier: Apache-2.0

//! Clocks read from the TSC inside the enclave.
//!
//! No clock reading the payload gets carries a value from the host. The host
//! only helps measure how fast the TSC counts, once, while the keep starts
//! (see `Handler::clock_init`); after that the clocks are computed in the
//! enclave, without leaving it:
//!
//! * the monotonic clock reads 0 when the clocks are anchored;
//! * the real-time clock reads the epoch the workload declares
//!   ([`declare_epoch`], from the keep's configuration) at the moment it
//!   declares it, and counts from the Unix epoch at the anchor until then.
//!   The date a build sees is thus one of its declared inputs, not a value
//!   the host picked.
//!
//! Both advance with the TSC. The host can move the TSC (`IA32_TSC_ADJUST`,
//! or the TSC offset of a virtual machine), so the time elapsed since the
//! anchor is clamped: no reading, on any thread, is less than one returned
//! before. How fast the clocks advance is all the host decides, as it decides
//! the schedule.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Lowest TSC frequency the clocks accept. The frequency comes from the host
/// (CPUID leaf 0x15, or a measurement against the host's clock); one outside
/// [`TSC_HZ_MIN`]..=[`TSC_HZ_MAX`] is a host lying.
pub const TSC_HZ_MIN: u64 = 100_000_000;

/// Highest TSC frequency the clocks accept (see [`TSC_HZ_MIN`]).
pub const TSC_HZ_MAX: u64 = 20_000_000_000;

const NSEC_PER_SEC: u128 = 1_000_000_000;

/// The clocks of the keep.
pub struct Clock {
    ready: AtomicBool,
    tsc_hz: AtomicU64,
    base_tsc: AtomicU64,
    /// The most nanoseconds since the anchor that any reading returned.
    last_ns: AtomicU64,
    /// How far the real-time clock is ahead of the monotonic clock, in two's
    /// complement: behind it if the declared epoch is earlier than the time
    /// elapsed since the anchor.
    realtime_offset: AtomicU64,
    declared: AtomicBool,
}

impl Clock {
    /// Clocks that are not anchored yet.
    pub const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            tsc_hz: AtomicU64::new(0),
            base_tsc: AtomicU64::new(0),
            last_ns: AtomicU64::new(0),
            realtime_offset: AtomicU64::new(0),
            declared: AtomicBool::new(false),
        }
    }

    /// Whether [`Clock::init`] has run.
    #[inline]
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Anchor the clocks at TSC value `tsc`, the TSC counting `tsc_hz` per
    /// second. Returns false, and leaves the clocks alone, if that frequency
    /// is out of range.
    pub fn init(&self, tsc_hz: u64, tsc: u64) -> bool {
        if !(TSC_HZ_MIN..=TSC_HZ_MAX).contains(&tsc_hz) {
            return false;
        }
        self.tsc_hz.store(tsc_hz, Ordering::Relaxed);
        self.base_tsc.store(tsc, Ordering::Relaxed);
        self.ready.store(true, Ordering::Release);
        true
    }

    /// Nanoseconds since the anchor at TSC value `tsc`, never fewer than a
    /// reading returned before. Only valid once the clocks are anchored.
    #[inline]
    pub fn elapsed_ns(&self, tsc: u64) -> u64 {
        let ticks = tsc.saturating_sub(self.base_tsc.load(Ordering::Relaxed)) as u128;
        let ns = ticks * NSEC_PER_SEC / self.tsc_hz.load(Ordering::Relaxed) as u128;
        let ns = ns.min(u64::MAX as u128) as u64;
        // Most readings are later than the last one; only those write.
        let last = self.last_ns.load(Ordering::Relaxed);
        if ns <= last {
            return last;
        }
        self.last_ns.fetch_max(ns, Ordering::Relaxed).max(ns)
    }

    /// Nanoseconds of the monotonic (`realtime == false`) or real-time clock
    /// at TSC value `tsc`. Only valid once the clocks are anchored.
    #[inline]
    pub fn at(&self, tsc: u64, realtime: bool) -> u64 {
        let ns = self.elapsed_ns(tsc);
        if realtime {
            ns.wrapping_add(self.realtime_offset.load(Ordering::Relaxed))
        } else {
            ns
        }
    }

    /// Have the real-time clock read `epoch_ns` (nanoseconds since the Unix
    /// epoch) at TSC value `tsc`. An epoch is declared once: returns false if
    /// one was declared before. Only valid once the clocks are anchored.
    pub fn declare_epoch(&self, epoch_ns: u64, tsc: u64) -> bool {
        if self.declared.swap(true, Ordering::SeqCst) {
            return false;
        }
        let now = self.elapsed_ns(tsc);
        self.realtime_offset
            .store(epoch_ns.wrapping_sub(now), Ordering::Relaxed);
        true
    }

    /// The monotonic-clock time at which the real-time clock reads
    /// `realtime_ns` (0 if that was before the anchor).
    pub fn realtime_to_monotonic(&self, realtime_ns: u64) -> u64 {
        let offset = self.realtime_offset.load(Ordering::Relaxed) as i64 as i128;
        (realtime_ns as i128 - offset).clamp(0, u64::MAX as i128) as u64
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// The keep's clocks.
static CLOCK: Clock = Clock::new();

/// Read the time stamp counter.
#[inline]
pub fn rdtsc() -> u64 {
    // Safety: RDTSC is allowed in SGX2 enclaves and has no side effects.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Whether [`init`] has run.
#[inline]
pub fn ready() -> bool {
    CLOCK.ready()
}

/// Anchor the clocks now, the TSC counting `tsc_hz` per second. Returns
/// false if that frequency is out of range.
pub fn init(tsc_hz: u64) -> bool {
    CLOCK.init(tsc_hz, rdtsc())
}

/// Nanoseconds of the monotonic (`realtime == false`) or real-time clock.
/// Only valid once [`ready`] returns true.
#[inline]
pub fn now_ns(realtime: bool) -> u64 {
    CLOCK.at(rdtsc(), realtime)
}

/// Have the real-time clock read `epoch_ns` now; once (see
/// [`Clock::declare_epoch`]). Only valid once [`ready`] returns true.
pub fn declare_epoch(epoch_ns: u64) -> bool {
    CLOCK.declare_epoch(epoch_ns, rdtsc())
}

/// The monotonic-clock time at which the real-time clock reads
/// `realtime_ns`.
pub fn realtime_to_monotonic(realtime_ns: u64) -> u64 {
    CLOCK.realtime_to_monotonic(realtime_ns)
}

#[cfg(test)]
mod tests {
    use super::{Clock, TSC_HZ_MAX, TSC_HZ_MIN};

    const GHZ: u64 = 1_000_000_000;
    const SEC: u64 = 1_000_000_000;

    fn clock(base_tsc: u64) -> Clock {
        let c = Clock::new();
        assert!(c.init(GHZ, base_tsc));
        c
    }

    #[test]
    fn rejects_frequencies_out_of_range() {
        let c = Clock::new();
        for hz in [0, 1, TSC_HZ_MIN - 1, TSC_HZ_MAX + 1, u64::MAX] {
            assert!(!c.init(hz, 0), "{hz} Hz");
            assert!(!c.ready());
        }
        assert!(c.init(TSC_HZ_MIN, 0));
        assert!(c.init(TSC_HZ_MAX, 0));
    }

    #[test]
    fn monotonic_starts_at_zero_and_realtime_at_the_unix_epoch() {
        let c = clock(1000);
        assert_eq!(c.at(1000, false), 0);
        assert_eq!(c.at(1000 + 3 * GHZ, false), 3 * SEC);
        assert_eq!(c.at(1000 + 4 * GHZ, true), 4 * SEC);
    }

    #[test]
    fn readings_never_decrease_when_the_tsc_moves_back() {
        let c = clock(1_000 * GHZ);
        assert_eq!(c.at(1_010 * GHZ, false), 10 * SEC);
        // The host set the TSC back by 5 s, then to before the anchor.
        assert_eq!(c.at(1_005 * GHZ, false), 10 * SEC);
        assert_eq!(c.at(900 * GHZ, false), 10 * SEC);
        assert_eq!(c.at(1_005 * GHZ, true), 10 * SEC);
        assert_eq!(c.at(1_011 * GHZ, false), 11 * SEC);
    }

    #[test]
    fn a_huge_tsc_jump_saturates() {
        let c = Clock::new();
        assert!(c.init(TSC_HZ_MIN, 0));
        assert_eq!(c.at(u64::MAX, false), u64::MAX);
        assert_eq!(c.at(1, false), u64::MAX);
    }

    #[test]
    fn the_declared_epoch_is_what_realtime_reads_then() {
        let c = clock(0);
        let epoch = 1_760_659_200 * SEC;
        assert!(c.declare_epoch(epoch, 2 * GHZ));
        assert_eq!(c.at(2 * GHZ, true), epoch);
        assert_eq!(c.at(5 * GHZ, true), epoch + 3 * SEC);
        assert_eq!(c.at(5 * GHZ, false), 5 * SEC);
        assert_eq!(c.realtime_to_monotonic(epoch + 3 * SEC), 5 * SEC);
        assert_eq!(c.realtime_to_monotonic(0), 0);
    }

    #[test]
    fn an_epoch_is_declared_once() {
        let c = clock(0);
        assert!(c.declare_epoch(100 * SEC, 0));
        assert!(!c.declare_epoch(200 * SEC, GHZ));
        assert_eq!(c.at(GHZ, true), 101 * SEC);
    }

    #[test]
    fn an_epoch_earlier_than_the_uptime_reads_correctly() {
        let c = clock(0);
        // Declared as 0 after 7 s: real time is 7 s behind monotonic time.
        assert!(c.declare_epoch(0, 7 * GHZ));
        assert_eq!(c.at(7 * GHZ, true), 0);
        assert_eq!(c.at(9 * GHZ, true), 2 * SEC);
        assert_eq!(c.realtime_to_monotonic(2 * SEC), 9 * SEC);
    }
}
