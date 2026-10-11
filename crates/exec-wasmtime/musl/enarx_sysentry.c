/* SPDX-License-Identifier: Apache-2.0 */
/* The shim's direct syscall entry, or 0 for real syscalls (see enarx_syscall.h). */
__attribute__((__visibility__("hidden"))) unsigned long __enarx_sysentry;
