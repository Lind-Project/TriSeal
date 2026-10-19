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
