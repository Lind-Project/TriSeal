// SPDX-License-Identifier: Apache-2.0

//! Committed pages freed by `munmap()` and not yet zeroed. They are zeroed
//! when a new mapping hands them out, so that freeing an evicted page does
//! not page it back in. One bit per page of the enclave address range.

use core::sync::atomic::{AtomicU64, Ordering};

use primordial::Page;

use crate::{committed, shim_address, ENCL_SIZE};

const WORDS: usize = ENCL_SIZE / Page::SIZE / 64;

static BITS: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];

#[inline]
fn index(addr: usize) -> (usize, u64) {
    let page = (addr - shim_address()) / Page::SIZE;
    (page / 64, 1 << (page % 64))
}

/// Record the committed page at `addr` as freed without being zeroed.
#[inline]
pub fn insert(addr: usize) {
    let (word, bit) = index(addr);
    BITS[word].fetch_or(bit, Ordering::Release);
}

/// Forget the page at `addr`, when it is given back to the host.
#[inline]
pub fn remove(addr: usize) {
    let (word, bit) = index(addr);
    BITS[word].fetch_and(!bit, Ordering::Release);
}

/// Zero every dirty page in `[start, end)` and clear its mark. Called without
/// the HEAP lock, on a range just handed out and not yet returned. Only pages
/// still committed are touched; `trim_unlocked` clears the mark of the rest.
pub fn zero_range(start: usize, end: usize) {
    let mut addr = start;
    while addr < end {
        let (word, bit) = index(addr);
        let bits = BITS[word].load(Ordering::Acquire);
        if bits == 0 && bit == 1 {
            addr += 64 * Page::SIZE;
            continue;
        }
        if bits & bit != 0 {
            if committed::contains(addr) {
                // Safety: the page is committed and belongs to the mapping
                // being handed out.
                unsafe { core::ptr::write_bytes(addr as *mut u8, 0, Page::SIZE) };
            }
            BITS[word].fetch_and(!bit, Ordering::Release);
        }
        addr += Page::SIZE;
    }
}
