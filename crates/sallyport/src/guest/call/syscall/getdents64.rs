// SPDX-License-Identifier: Apache-2.0

// getdents64: fills `buf` with linux_dirent64 records; byte-buffer Output exactly like read.

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collector, Output};
use crate::libc::SYS_getdents64;
use crate::Result;

use core::ffi::{c_int, c_long, c_size_t};

pub struct Getdents64<'a> {
    pub fd: c_int,
    pub buf: &'a mut [u8],
}

unsafe impl<'a> Alloc<'a> for Getdents64<'a> {
    const NUM: c_long = SYS_getdents64;

    type Argv = Argv<3>;
    type Ret = c_size_t;

    type Staged = Output<'a, [u8], &'a mut [u8]>;
    type Committed = Self::Staged;
    type Collected = Option<Result<c_size_t>>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let (buf, _) = Output::stage_slice_max(alloc, self.buf)?;
        Ok((Argv([self.fd as _, buf.offset(), buf.len()]), buf))
    }

    fn collect(
        buf: Self::Committed,
        ret: Result<Self::Ret>,
        col: &impl Collector,
    ) -> Self::Collected {
        match ret {
            Ok(ret) if ret > buf.len() => None,
            res @ Ok(ret) => {
                unsafe { buf.collect_range(col, 0..ret) };
                Some(res)
            }
            err => Some(err),
        }
    }
}
