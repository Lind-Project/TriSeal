// SPDX-License-Identifier: Apache-2.0

//! Pages of the enclave that have been committed (added by the host with
//! ENCLS[EAUG] and accepted by the shim with ENCLU[EACCEPTCOPY]).
//!
//! The shim never gives a committed page back to the host: `munmap()` zeroes
//! it and leaves it committed, so that it can be handed out again without
//! leaving the enclave. One bit per page of the enclave address range.

use core::sync::atomic::{AtomicU64, Ordering};

use primordial::Page;

use crate::{shim_address, ENCL_SIZE};

const WORDS: usize = ENCL_SIZE / Page::SIZE / 64;

static BITS: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];

#[inline]
fn index(addr: usize) -> (usize, u64) {
    let page = (addr - shim_address()) / Page::SIZE;
    (page / 64, 1 << (page % 64))
}

/// Whether the page at `addr` is committed.
#[inline]
pub fn contains(addr: usize) -> bool {
    let (word, bit) = index(addr);
    BITS[word].load(Ordering::Acquire) & bit != 0
}

/// Record the page at `addr` as committed.
#[inline]
pub fn insert(addr: usize) {
    let (word, bit) = index(addr);
    BITS[word].fetch_or(bit, Ordering::Release);
}

/// Record the page at `addr` as no longer committed.
#[inline]
pub fn remove(addr: usize) {
    let (word, bit) = index(addr);
    BITS[word].fetch_and(!bit, Ordering::Release);
}

/// Call `f` with the address of every committed page in `[start, end)`.
pub fn for_each(start: usize, end: usize, mut f: impl FnMut(usize)) {
    let mut addr = start;
    while addr < end {
        let (word, bit) = index(addr);
        let bits = BITS[word].load(Ordering::Acquire);
        if bits == 0 && bit == 1 {
            // Skip a whole empty word.
            addr += 64 * Page::SIZE;
            continue;
        }
        if bits & bit != 0 {
            f(addr);
        }
        addr += Page::SIZE;
    }
}

/// Pages in use, for measuring the payload's working set (feature `in-use`,
/// which `working-set` turns on): committed pages that lie in a mapping. Committed pages
/// outside any mapping (unmapped and zeroed, kept for reuse) do not count.
/// The shim cannot see whether a page that is already committed is touched
/// again, so two counts bound what is in use:
///
/// - touched: pages committed for the mapping they lie in, that is, first
///   touched by it when the shim commits on first touch only (feature
///   `working-set`; with commits ahead, committed for it) (the lower bound);
/// - mapped: those, and committed pages a new mapping reuses, whether it
///   touches them or not (the upper bound).
///
/// Each has its current value and its peak; without the feature they stay 0.
pub mod in_use {
    use core::sync::atomic::AtomicU64;

    /// Pages in use now, lower bound.
    pub static TOUCHED: AtomicU64 = AtomicU64::new(0);
    /// Peak of [`TOUCHED`].
    pub static TOUCHED_PEAK: AtomicU64 = AtomicU64::new(0);
    /// Pages in use now, upper bound.
    pub static MAPPED: AtomicU64 = AtomicU64::new(0);
    /// Peak of [`MAPPED`].
    pub static MAPPED_PEAK: AtomicU64 = AtomicU64::new(0);

    #[cfg(feature = "in-use")]
    mod bits {
        use core::sync::atomic::{AtomicU64, Ordering};

        use super::super::{index, WORDS};
        use super::{MAPPED, MAPPED_PEAK, TOUCHED, TOUCHED_PEAK};

        static TOUCHED_BITS: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];
        static MAPPED_BITS: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];

        fn set(bits: &[AtomicU64; WORDS], count: &AtomicU64, peak: &AtomicU64, addr: usize) {
            let (word, bit) = index(addr);
            if bits[word].fetch_or(bit, Ordering::Relaxed) & bit == 0 {
                let now = count.fetch_add(1, Ordering::Relaxed) + 1;
                peak.fetch_max(now, Ordering::Relaxed);
            }
        }

        fn clear(bits: &[AtomicU64; WORDS], count: &AtomicU64, addr: usize) {
            let (word, bit) = index(addr);
            if bits[word].fetch_and(!bit, Ordering::Relaxed) & bit != 0 {
                count.fetch_sub(1, Ordering::Relaxed);
            }
        }

        pub fn committed(addr: usize) {
            set(&TOUCHED_BITS, &TOUCHED, &TOUCHED_PEAK, addr);
            set(&MAPPED_BITS, &MAPPED, &MAPPED_PEAK, addr);
        }

        pub fn mapped(addr: usize) {
            set(&MAPPED_BITS, &MAPPED, &MAPPED_PEAK, addr);
        }

        pub fn unmapped(addr: usize) {
            clear(&TOUCHED_BITS, &TOUCHED, addr);
            clear(&MAPPED_BITS, &MAPPED, addr);
        }
    }

    /// The page at `addr` was just committed for the mapping it lies in.
    #[inline]
    pub fn committed(_addr: usize) {
        #[cfg(feature = "in-use")]
        bits::committed(_addr);
    }

    /// `[start, end)` was just mapped: count the committed pages it reuses.
    #[inline]
    pub fn mapped(_start: usize, _end: usize) {
        #[cfg(feature = "in-use")]
        super::for_each(_start, _end, bits::mapped);
    }

    /// The committed page at `addr` is no longer mapped.
    #[inline]
    pub fn unmapped(_addr: usize) {
        #[cfg(feature = "in-use")]
        bits::unmapped(_addr);
    }
}
