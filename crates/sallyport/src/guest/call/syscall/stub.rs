// SPDX-License-Identifier: Apache-2.0

use super::super::Stub;
use crate::guest::alloc::Collector;
use crate::libc::{
    gid_t, pid_t, sigset_t, stack_t, uid_t, utsname, EAGAIN, EINVAL, ENOENT, GRND_NONBLOCK,
    GRND_RANDOM,
};
use crate::Result;

use core::ffi::{c_char, c_int, c_size_t, c_uint};

/// Fake GID returned by enarx.
pub const FAKE_GID: gid_t = 1000;

/// Fake PID returned by enarx.
pub const FAKE_PID: pid_t = 1000;

/// Fake TID returned by enarx.
pub const FAKE_TID: pid_t = 1;

/// Fake UID returned by enarx.
pub const FAKE_UID: uid_t = 1000;

// `Fstat` moved to `super::fstat` — it is no longer a pure stub. The standard-stream
// special case is preserved there; every other descriptor is now proxied to the host.

pub struct Getegid;

impl Stub for Getegid {
    type Ret = gid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_GID
    }
}

pub struct Geteuid;

impl Stub for Geteuid {
    type Ret = uid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_UID
    }
}

pub struct Getgid;

impl Stub for Getgid {
    type Ret = gid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_GID
    }
}

pub struct Getpid;

impl Stub for Getpid {
    type Ret = pid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_PID
    }
}

pub struct Getrandom<'a> {
    pub buf: &'a mut [u8],
    pub flags: c_uint,
}

impl Stub for Getrandom<'_> {
    type Ret = Result<c_size_t>;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        if self.flags & !(GRND_NONBLOCK | GRND_RANDOM) != 0 {
            return Err(EINVAL);
        }

        for (i, chunk) in self.buf.chunks_mut(8).enumerate() {
            let mut el = 0u64;
            loop {
                if unsafe { core::arch::x86_64::_rdrand64_step(&mut el) } == 1 {
                    chunk.copy_from_slice(&el.to_ne_bytes()[..chunk.len()]);
                    break;
                } else {
                    if (self.flags & GRND_NONBLOCK) != 0 {
                        return Err(EAGAIN);
                    }
                    if (self.flags & GRND_RANDOM) != 0 {
                        return Ok(i.checked_mul(8).unwrap());
                    }
                }
            }
        }
        Ok(self.buf.len())
    }
}

pub struct Getuid;

impl Stub for Getuid {
    type Ret = uid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_UID
    }
}

pub struct Readlink<'a> {
    pub pathname: &'a [u8],
    pub buf: &'a mut [u8],
}

impl Stub for Readlink<'_> {
    type Ret = Option<Result<c_size_t>>;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        match self.pathname {
            b"/proc/self/exe\0" => {
                const DEST: &[u8; 6] = b"/init\0";
                if self.buf.len() < DEST.len() {
                    return Some(Err(EINVAL));
                }
                self.buf[..DEST.len()].copy_from_slice(DEST);
                Some(Ok(DEST.len()))
            }
            _ => Some(Err(ENOENT)),
        }
    }
}

pub struct RtSigprocmask<'a> {
    pub how: c_int,
    pub set: Option<&'a sigset_t>,
    pub oldset: Option<&'a mut sigset_t>,
    pub sigsetsize: c_size_t,
}

impl Stub for RtSigprocmask<'_> {
    type Ret = Result<()>;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        Ok(())
    }
}

pub struct Sigaltstack<'a> {
    pub ss: Option<&'a stack_t>,
    pub old_ss: Option<&'a mut stack_t>,
}

impl Stub for Sigaltstack<'_> {
    type Ret = Result<()>;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        Ok(())
    }
}

pub struct SetTidAddress<'a> {
    pub tidptr: &'a mut c_int,
}

impl Stub for SetTidAddress<'_> {
    type Ret = pid_t;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        FAKE_TID
    }
}

pub struct Uname<'a> {
    pub buf: &'a mut utsname,
}

impl Stub for Uname<'_> {
    type Ret = Result<()>;

    fn collect(self, _: &impl Collector) -> Self::Ret {
        fn fill(buf: &mut [c_char; 65], with: &str) {
            let src = with.as_bytes();
            for (i, b) in buf.iter_mut().enumerate() {
                *b = *src.get(i).unwrap_or(&0) as _;
            }
        }
        fill(&mut self.buf.sysname, "Linux");
        fill(&mut self.buf.nodename, "localhost.localdomain");
        fill(&mut self.buf.release, "5.6.0");
        fill(&mut self.buf.version, "#1");
        fill(&mut self.buf.machine, "x86_64");
        Ok(())
    }
}
