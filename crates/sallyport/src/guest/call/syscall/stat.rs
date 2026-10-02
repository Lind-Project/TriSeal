// SPDX-License-Identifier: Apache-2.0

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collect, Collector, Input, Output};
use crate::libc::{stat, SYS_stat};
use crate::Result;

use core::ffi::c_long;

pub struct Stat<'a> {
    pub pathname: &'a [u8],
    pub statbuf: &'a mut stat,
}

unsafe impl<'a> Alloc<'a> for Stat<'a> {
    const NUM: c_long = SYS_stat;

    type Argv = Argv<3>;
    type Ret = ();

    type Staged = (Input<'a, [u8], &'a [u8]>, Output<'a, stat, &'a mut stat>);
    type Committed = ((), Output<'a, stat, &'a mut stat>);
    type Collected = Result<()>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let pathname = Input::stage_slice(alloc, self.pathname)?;
        let statbuf = Output::stage(alloc, self.statbuf)?;
        Ok((
            Argv([pathname.offset(), pathname.len(), statbuf.offset()]),
            (pathname, statbuf),
        ))
    }

    fn collect(
        (_, statbuf): Self::Committed,
        ret: Result<Self::Ret>,
        col: &impl Collector,
    ) -> Self::Collected {
        if ret.is_ok() {
            statbuf.collect(col);
        }
        ret
    }
}
