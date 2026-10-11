// SPDX-License-Identifier: Apache-2.0

use super::super::alloc::kind;
use super::super::types::Argv;
use super::super::{MaybeAlloc, UnstagedMaybeAlloc};
use super::Alloc;
use crate::guest::alloc::{Allocator, Collect, Collector, Output};
use crate::libc::{stat, SYS_fstat, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO, S_IFIFO};
use crate::Result;

use core::ffi::{c_int, c_long};
use core::mem;

pub struct Fstat<'a> {
    pub fd: c_int,
    pub statbuf: &'a mut stat,
}

impl<'a> MaybeAlloc<'a, kind::Syscall> for Fstat<'a> {
    type Alloc = AllocFstat<'a>;

    #[inline]
    fn stage(self) -> Result<UnstagedMaybeAlloc<'a, kind::Syscall, Self::Alloc>> {
        match self.fd {
            // Standard streams are synthesised in-enclave: the host's notion of these
            // descriptors is not necessarily the guest's, and enarx has always reported
            // them as pipes. Preserved verbatim from the previous stub implementation.
            STDIN_FILENO | STDOUT_FILENO | STDERR_FILENO => {
                #[allow(clippy::integer_arithmetic)]
                const fn makedev(x: u64, y: u64) -> u64 {
                    (((x) & 0xffff_f000u64) << 32)
                        | (((x) & 0x0000_0fffu64) << 8)
                        | (((y) & 0xffff_ff00u64) << 12)
                        | ((y) & 0x0000_00ffu64)
                }

                let mut p: stat = unsafe { mem::zeroed() };

                p.st_dev = makedev(
                    0,
                    match self.fd {
                        0 => 0x19,
                        _ => 0xc,
                    },
                );
                p.st_ino = 3;
                p.st_mode = S_IFIFO | 0o600;
                p.st_nlink = 1;
                p.st_uid = 1000;
                p.st_gid = 5;
                p.st_blksize = 4096;
                p.st_blocks = 0;
                p.st_rdev = makedev(0x88, 0);
                p.st_size = 0;

                p.st_atime = 1_579_507_218 /* 2020-01-21T11:45:08.467721685+0100 */;
                p.st_atime_nsec = 0;
                p.st_mtime = 1_579_507_218 /* 2020-01-21T11:45:07.467721685+0100 */;
                p.st_mtime_nsec = 0;
                p.st_ctime = 1_579_507_218 /* 2020-01-20T09:00:18.467721685+0100 */;
                p.st_ctime_nsec = 0;

                *self.statbuf = p;
                Ok(UnstagedMaybeAlloc::Stub(Ok(())))
            }
            // Any other descriptor refers to a real host file: proxy it.
            _ => Ok(UnstagedMaybeAlloc::Alloc(AllocFstat(self))),
        }
    }
}

pub struct AllocFstat<'a>(Fstat<'a>);

unsafe impl<'a> Alloc<'a> for AllocFstat<'a> {
    const NUM: c_long = SYS_fstat;

    type Argv = Argv<2>;
    type Ret = ();

    type Staged = Output<'a, stat, &'a mut stat>;
    type Committed = Self::Staged;
    type Collected = Result<()>;

    fn stage(self, alloc: &mut impl Allocator) -> Result<(Self::Argv, Self::Staged)> {
        let statbuf = Output::stage(alloc, self.0.statbuf)?;
        Ok((Argv([self.0.fd as _, statbuf.offset()]), statbuf))
    }

    fn collect(
        statbuf: Self::Committed,
        ret: Result<Self::Ret>,
        col: &impl Collector,
    ) -> Self::Collected {
        if ret.is_ok() {
            statbuf.collect(col);
        }
        ret
    }
}
