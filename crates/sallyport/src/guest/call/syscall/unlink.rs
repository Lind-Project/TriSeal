// SPDX-License-Identifier: Apache-2.0

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collector, Input};
use crate::libc::SYS_unlink;
use crate::Result;

use core::ffi::{c_int, c_long};

pub struct Unlink<'a> {
    pub pathname: &'a [u8],
}

unsafe impl<'a> Alloc<'a> for Unlink<'a> {
    const NUM: c_long = SYS_unlink;

    type Argv = Argv<2>;
    type Ret = c_int;

    type Staged = Input<'a, [u8], &'a [u8]>;
    type Committed = ();
    type Collected = Result<c_int>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let pathname = Input::stage_slice(alloc, self.pathname)?;
        Ok((Argv([pathname.offset(), pathname.len()]), pathname))
    }

    fn collect(_: Self::Committed, ret: Result<Self::Ret>, _: &impl Collector) -> Self::Collected {
        ret
    }
}
