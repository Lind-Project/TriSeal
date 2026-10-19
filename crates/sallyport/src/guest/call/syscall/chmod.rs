// SPDX-License-Identifier: Apache-2.0

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collector, Input};
use crate::libc::{mode_t, SYS_chmod};
use crate::Result;

use core::ffi::{c_int, c_long};

pub struct Chmod<'a> {
    pub pathname: &'a [u8],
    pub mode: mode_t,
}

unsafe impl<'a> Alloc<'a> for Chmod<'a> {
    const NUM: c_long = SYS_chmod;

    type Argv = Argv<3>;
    type Ret = c_int;

    type Staged = Input<'a, [u8], &'a [u8]>;
    type Committed = ();
    type Collected = Result<c_int>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let pathname = Input::stage_slice(alloc, self.pathname)?;
        Ok((
            Argv([pathname.offset(), pathname.len(), self.mode as _]),
            pathname,
        ))
    }

    fn collect(_: Self::Committed, ret: Result<Self::Ret>, _: &impl Collector) -> Self::Collected {
        ret
    }
}
