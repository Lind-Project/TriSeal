// SPDX-License-Identifier: Apache-2.0

use super::super::types::Argv;
use super::Alloc;
use crate::guest::alloc::{Allocator, Collect, Collector, Output};
use crate::libc::SYS_pipe2;
use crate::Result;

use core::ffi::{c_int, c_long};

/// `pipe2` cannot be a passthrough: the host must write the two descriptors
/// back into enclave memory, so they are staged as an `Output` in the block.
pub struct Pipe2<'a> {
    pub pipefd: &'a mut [c_int; 2],
    pub flags: c_int,
}

unsafe impl<'a> Alloc<'a> for Pipe2<'a> {
    const NUM: c_long = SYS_pipe2;

    type Argv = Argv<2>;
    type Ret = c_int;

    type Staged = Output<'a, [c_int; 2], &'a mut [c_int; 2]>;
    type Committed = Self::Staged;
    type Collected = Result<c_int>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let pipefd = Output::stage(alloc, self.pipefd)?;
        Ok((Argv([pipefd.offset(), self.flags as _]), pipefd))
    }

    fn collect(
        pipefd: Self::Committed,
        ret: Result<Self::Ret>,
        col: &impl Collector,
    ) -> Self::Collected {
        if ret.is_ok() {
            pipefd.collect(col);
        }
        ret
    }
}
