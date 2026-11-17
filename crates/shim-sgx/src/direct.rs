// SPDX-License-Identifier: Apache-2.0

//! Syscalls without an exception.
//!
//! The payload's `syscall` instruction is illegal in an enclave: it raises
//! #UD, the host enters the enclave again to run the shim's handler, and the
//! handler's return leaves the enclave once more before the payload resumes.
//! A payload linked against a libc that calls [`enarx_direct_entry`] in
//! place of `syscall` gets the same handler without leaving the enclave.
//!
//! The libc does what `syscall` does to the registers: it loads the address
//! to return to into `rcx` and jumps here with the syscall number in `rax`
//! and the arguments in `rdi`, `rsi`, `rdx`, `r10`, `r8` and `r9`, without
//! touching the stack. The entry saves every register into a frame laid out
//! like the state save area of an exception, as if the `syscall` instruction
//! were at the return address minus 2, and runs the #UD handler on it. The
//! handler leaves the frame as it would leave the state save area: the
//! result in `rax` and `rip` past the instruction, or another thread's state
//! entirely (`exit` returns to the shim's thread loop). The entry then loads
//! the frame and jumps to it, which for an ordinary syscall is a return to
//! the caller with `rcx` and `r11` clobbered, as after `syscall`.
//!
//! The frame and the handler run on the shim's stack for exceptions of the
//! calling thread (found from its TCB, at its GS base), 1 MiB below where an
//! exception handler starts, so that one taken meanwhile has room above.
//! Not on the caller's stack: musl's `__unmapself` unmaps the stack a thread
//! is running on, and the shim zeroes those pages, frame and all. The shim
//! is built without SSE, so the caller's vector registers are left alone.

use core::arch::global_asm;
use core::mem::{offset_of, size_of};

use sgx::ssa::GenPurposeRegs;

use crate::CSSA_0_STACK_SIZE;

/// Room left above the direct entry's stack for exception handlers.
const HANDLER_ROOM: usize = 1 << 20;

use crate::handler::handle_direct;

extern "C" {
    /// The direct syscall entry (see the module documentation).
    pub fn enarx_direct_entry();
}

global_asm!(
    ".pushsection .text,\"ax\",@progbits",
    ".global enarx_direct_entry",
    ".type enarx_direct_entry,@function",
    "enarx_direct_entry:",
    "mov    r11,    rsp                 ", // r11 = caller's rsp
    "rdgsbase rsp                       ", // rsp = the TCB
    "sub    rsp,    {BELOW_TCB}         ", // below the CSSA 0 stack and room
    "and    rsp,    ~0xf                ",
    "sub    rsp,    {SIZE}              ", // the frame
    "mov    [rsp + {RAX}], rax          ",
    "mov    [rsp + {RCX}], rcx          ", // the return address, as syscall leaves it
    "mov    [rsp + {RDX}], rdx          ",
    "mov    [rsp + {RBX}], rbx          ",
    "mov    [rsp + {RSP}], r11          ",
    "mov    [rsp + {RBP}], rbp          ",
    "mov    [rsp + {RSI}], rsi          ",
    "mov    [rsp + {RDI}], rdi          ",
    "mov    [rsp + {R8}],  r8           ",
    "mov    [rsp + {R9}],  r9           ",
    "mov    [rsp + {R10}], r10          ",
    "mov    [rsp + {R11}], r11          ",
    "mov    [rsp + {R12}], r12          ",
    "mov    [rsp + {R13}], r13          ",
    "mov    [rsp + {R14}], r14          ",
    "mov    [rsp + {R15}], r15          ",
    "pushfq                             ",
    "pop    rbx                         ",
    "mov    [rsp + {RFLAGS}], rbx       ",
    "lea    rbx,    [rcx - 2]           ", // where `syscall` would have been
    "mov    [rsp + {RIP}], rbx          ",
    "rdfsbase rbx                       ",
    "mov    [rsp + {FSBASE}], rbx       ",
    "rdgsbase rbx                       ",
    "mov    [rsp + {GSBASE}], rbx       ",
    "cld                                ",
    "mov    rdi,    rsp                 ",
    "call   {HANDLE}                    ",
    // Load the frame the handler left and jump to it.
    "mov    r11,    rsp                 ",
    "mov    rax,    [r11 + {FSBASE}]    ",
    "wrfsbase rax                       ",
    "mov    rax,    [r11 + {GSBASE}]    ",
    "wrgsbase rax                       ",
    "push   QWORD PTR [r11 + {RFLAGS}]  ",
    "popfq                              ",
    "mov    rcx,    [r11 + {RCX}]       ",
    "mov    rdx,    [r11 + {RDX}]       ",
    "mov    rbx,    [r11 + {RBX}]       ",
    "mov    rbp,    [r11 + {RBP}]       ",
    "mov    rsi,    [r11 + {RSI}]       ",
    "mov    rdi,    [r11 + {RDI}]       ",
    "mov    r8,     [r11 + {R8}]        ",
    "mov    r9,     [r11 + {R9}]        ",
    "mov    r10,    [r11 + {R10}]       ",
    "mov    r12,    [r11 + {R12}]       ",
    "mov    r13,    [r11 + {R13}]       ",
    "mov    r14,    [r11 + {R14}]       ",
    "mov    r15,    [r11 + {R15}]       ",
    "mov    rax,    [r11 + {RAX}]       ",
    "mov    rsp,    [r11 + {RSP}]       ",
    "mov    r11,    [r11 + {RIP}]       ", // the handler moved it past `syscall`
    "jmp    r11                         ",
    ".size enarx_direct_entry, .-enarx_direct_entry",
    ".popsection",
    SIZE = const (size_of::<GenPurposeRegs>() + 15) & !15,
    BELOW_TCB = const CSSA_0_STACK_SIZE + HANDLER_ROOM,
    RAX = const offset_of!(GenPurposeRegs, rax),
    RCX = const offset_of!(GenPurposeRegs, rcx),
    RDX = const offset_of!(GenPurposeRegs, rdx),
    RBX = const offset_of!(GenPurposeRegs, rbx),
    RSP = const offset_of!(GenPurposeRegs, rsp),
    RBP = const offset_of!(GenPurposeRegs, rbp),
    RSI = const offset_of!(GenPurposeRegs, rsi),
    RDI = const offset_of!(GenPurposeRegs, rdi),
    R8 = const offset_of!(GenPurposeRegs, r8),
    R9 = const offset_of!(GenPurposeRegs, r9),
    R10 = const offset_of!(GenPurposeRegs, r10),
    R11 = const offset_of!(GenPurposeRegs, r11),
    R12 = const offset_of!(GenPurposeRegs, r12),
    R13 = const offset_of!(GenPurposeRegs, r13),
    R14 = const offset_of!(GenPurposeRegs, r14),
    R15 = const offset_of!(GenPurposeRegs, r15),
    RFLAGS = const offset_of!(GenPurposeRegs, rflags),
    RIP = const offset_of!(GenPurposeRegs, rip),
    FSBASE = const offset_of!(GenPurposeRegs, fsbase),
    GSBASE = const offset_of!(GenPurposeRegs, gsbase),
    HANDLE = sym direct_syscall,
);

/// Run the syscall handler on a frame saved by [`enarx_direct_entry`].
///
/// # Safety
///
/// Only called by [`enarx_direct_entry`], on a thread running the payload,
/// whose GS base points at its TCB.
unsafe extern "C" fn direct_syscall(gpr: &mut GenPurposeRegs) {
    handle_direct(gpr);
}
