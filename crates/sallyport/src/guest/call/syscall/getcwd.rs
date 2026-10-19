// SPDX-License-Identifier: Apache-2.0

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collector, Output};
use crate::libc::SYS_getcwd;
use crate::Result;

use core::ffi::{c_long, c_size_t};

pub struct Getcwd<'a> {
    pub buf: &'a mut [u8],
}

unsafe impl<'a> Alloc<'a> for Getcwd<'a> {
    const NUM: c_long = SYS_getcwd;

    type Argv = Argv<2>;
    type Ret = c_size_t;

    type Staged = Output<'a, [u8], &'a mut [u8]>;
    type Committed = Self::Staged;
    type Collected = Option<Result<c_size_t>>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let (buf, _) = Output::stage_slice_max(alloc, self.buf)?;
        Ok((Argv([buf.offset(), buf.len()]), buf))
    }

    fn collect(
        buf: Self::Committed,
        ret: Result<Self::Ret>,
        col: &impl Collector,
    ) -> Self::Collected {
        match ret {
            // The kernel returns the length of the path *including* the nul terminator;
            // a value larger than the buffer we staged means the host is lying to us.
            Ok(ret) if ret > buf.len() => None,
            res @ Ok(ret) => {
                unsafe { buf.collect_range(col, 0..ret) };
                Some(res)
            }
            err => Some(err),
        }
    }
}
