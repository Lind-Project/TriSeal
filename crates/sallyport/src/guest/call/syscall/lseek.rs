// SPDX-License-Identifier: Apache-2.0

use super::super::alloc::kind;
use super::super::types::Argv;
use super::super::{MaybeAlloc, UnstagedMaybeAlloc};
use super::PassthroughAlloc;
use crate::libc::SYS_lseek;
use crate::Result;

use core::ffi::{c_int, c_long};

pub struct Lseek {
    pub fd: c_int,
    pub offset: i64,
    pub whence: c_int,
}

impl<'a> MaybeAlloc<'a, kind::Syscall> for Lseek {
    type Alloc = AllocLseek;

    #[inline]
    fn stage(self) -> Result<UnstagedMaybeAlloc<'a, kind::Syscall, Self::Alloc>> {
        Ok(UnstagedMaybeAlloc::Alloc(AllocLseek(self)))
    }
}

pub struct AllocLseek(Lseek);

unsafe impl PassthroughAlloc for AllocLseek {
    const NUM: c_long = SYS_lseek;

    type Argv = Argv<3>;
    type Ret = usize;

    #[inline]
    fn stage(self) -> Self::Argv {
        Argv([self.0.fd as _, self.0.offset as _, self.0.whence as _])
    }
}
