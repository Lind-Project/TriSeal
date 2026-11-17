/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Syscalls through the Enarx SGX shim's direct entry.
 *
 * ENARX_SYSCALL takes the place of the `syscall` instruction in musl. If
 * the payload has set __enarx_sysentry (inside an SGX keep, to the shim's
 * entry), it loads the address to return to into rcx, as `syscall` does,
 * and jumps there without touching the stack: the shim handles the syscall
 * in the enclave and jumps back. Otherwise it is an ordinary `syscall`.
 * Like `syscall`, it clobbers rcx, r11 and the flags.
 */
#ifndef ENARX_SYSCALL_H
#define ENARX_SYSCALL_H

#ifdef __ASSEMBLER__

.macro ENARX_SYSCALL
	cmpq $0, __enarx_sysentry(%rip)
	je .Lenarx_native\@
	leaq .Lenarx_after\@(%rip), %rcx
	jmpq *__enarx_sysentry(%rip)
.Lenarx_native\@:
	syscall
.Lenarx_after\@:
.endm

#else

__asm__(
	".macro ENARX_SYSCALL\n"
	"cmpq $0, __enarx_sysentry(%rip)\n"
	"je .Lenarx_native\\@\n"
	"leaq .Lenarx_after\\@(%rip), %rcx\n"
	"jmpq *__enarx_sysentry(%rip)\n"
	".Lenarx_native\\@:\n"
	"syscall\n"
	".Lenarx_after\\@:\n"
	".endm\n"
);

#endif

#endif
