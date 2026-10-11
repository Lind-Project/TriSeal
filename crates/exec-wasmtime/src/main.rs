// SPDX-License-Identifier: Apache-2.0
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]
#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(rust_2018_idioms)]

use enarx_exec_wasmtime::execute;

/// Set FSBASE
///
/// Overwrite the only location in musl, which uses the `arch_prctl` syscall
#[no_mangle]
pub extern "C" fn __set_thread_area(p: *mut core::ffi::c_void) -> core::ffi::c_int {
    let mut rax: usize = 0;
    if unsafe { core::arch::x86_64::__cpuid(7).ebx } & 1 == 1 {
        unsafe {
            std::arch::asm!("wrfsbase {}", in(reg) p);
        }
    } else {
        const ARCH_SET_FS: core::ffi::c_int = 0x1002;
        unsafe {
            std::arch::asm!(
            "syscall",
            inlateout("rax")  libc::SYS_arch_prctl => rax,
            in("rdi") ARCH_SET_FS,
            in("rsi") p,
            lateout("rcx") _, // clobbered
            lateout("r11") _, // clobbered
            );
        }
    }
    rax as _
}

/// Set up the memory the keep shares with the host (console, phase page)
/// and, when linked against the musl from `musl/build.sh`, send every
/// syscall to the SGX shim's direct entry from now on. Outside an SGX keep
/// the call fails and syscalls stay ordinary syscalls.
fn direct_init() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use sallyport::item::enarxcall::SYS_DIRECT_INIT;
        let entry = unsafe { libc::syscall(SYS_DIRECT_INIT as libc::c_long) };
        #[cfg(enarx_direct_musl)]
        if entry > 0 {
            extern "C" {
                static mut __enarx_sysentry: usize;
            }
            // Safety: no other thread runs yet; the threads started later
            // see the entry.
            unsafe { __enarx_sysentry = entry as usize };
        }
        let _ = entry;
    }
}

fn main() -> anyhow::Result<()> {
    direct_init();
    execute()
}
