// SPDX-License-Identifier: Apache-2.0

//! FIXME: add docs

macro_rules! debug {
    ($dst:expr, $($arg:tt)*) => {
        #[allow(unused_must_use)] {
            if $crate::DEBUG {
                use core::fmt::Write;
                write!($dst, $($arg)*);
            }
        }
    };
}

macro_rules! debugln {
    ($dst:expr) => { debugln!($dst,) };
    ($dst:expr, $($arg:tt)*) => {
        if $crate::DEBUG {
            use core::fmt::Write;
            let _ = writeln!($dst, $($arg)*);
        }
    };
}

pub(crate) mod gdb;
pub(crate) mod key;
pub(crate) mod usermem;

use crate::console::CONSOLE;
use crate::handler::usermem::UserMemScope;
use crate::heap::{Access, Heap};
use crate::thread::{
    backoff, take_idle, NewThread, NewThreadFromRegisters, Tcb, Tcs, ThreadMem, NEW_THREAD_QUEUE,
    POOL_PENDING, POOL_STARTED, POOL_THREADS, THREADS_FREE, THREAD_ID_CNT,
};
use crate::{clock, committed};
use crate::{
    shim_address, CSSA_0_STACK_SIZE, CSSA_1_PLUS_STACK_SIZE, DEBUG, ENARX_EXEC_END,
    ENARX_EXEC_START, ENCL_SIZE, NUM_SSA,
};

use core::arch::asm;
use core::arch::x86_64::CpuidResult;
use core::ffi::{c_int, c_long, c_size_t, c_ulong, c_void};
use core::fmt::Write;
use core::mem::size_of;
use core::ops::Deref;
use core::ptr::read_unaligned;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use mmledger::{Record, Region, Span};
use primordial::{Address, Offset, Page};
use sallyport::guest::syscall::ClockGettime;
use sallyport::guest::{self, syscall as sys, Handler as _, Platform, ThreadLocalStorage};
use sallyport::item::enarxcall::sgx::{Report, ReportData, TargetInfo, TECH};
use sallyport::item::enarxcall::{SYS_GETATT, SYS_GETKEY};
use sallyport::libc::{
    clockid_t, off_t, pid_t, timespec, CloneFlags, SYS_clock_gettime, SYS_futex, SYS_sched_yield,
    CLOCK_MONOTONIC, EACCES, EAGAIN, EINVAL, EIO, EMSGSIZE, ENOMEM, ENOSYS, ENOTSUP,
    FUTEX_BITSET_MATCH_ANY, FUTEX_CLOCK_REALTIME, FUTEX_PRIVATE_FLAG, FUTEX_WAIT,
    FUTEX_WAIT_BITSET, FUTEX_WAKE, FUTEX_WAKE_BITSET, MAP_ANONYMOUS, MAP_PRIVATE, PROT_EXEC,
    PROT_READ, PROT_WRITE, STDERR_FILENO, STDOUT_FILENO,
};
use sallyport::Error;
use sgx::page::{Class, Flags};
use sgx::ssa::StateSaveArea;
use sgx::ssa::Vector;
use spin::{Lazy, RwLock};
use x86_64::addr::VirtAddr;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::paging::Page as PageAddr;

// Opcode constants, details in Volume 2 of the Intel 64 and IA-32 Architectures Software
// Developer's Manual
const OP_SYSCALL: u16 = 0x050f;
const OP_CPUID: u16 = 0xa20f;

/// The keep heap
pub static HEAP: Lazy<RwLock<Heap>> = Lazy::new(|| {
    let start = unsafe { &ENARX_EXEC_END as *const _ } as usize;
    let end = shim_address() + ENCL_SIZE;
    let span: Span = Region::new(Address::new(start), Address::new(end)).into();
    RwLock::new(Heap::new(span.start, span.count))
});

/// Whether the whole heap is mapped on the host. Once it is, the kernel adds
/// (EAUG) a page when the enclave first touches it, and committing a page
/// takes only an EACCEPTCOPY inside the enclave.
static HEAP_HOST_MAPPED: AtomicBool = AtomicBool::new(false);

/// Bumped by every futex wake. Waiters spin in the enclave until it or the
/// futex word changes, instead of parking their thread on the host.
static FUTEX_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Number of threads waiting on each futex word, so that a wake can report
/// how many threads it reached.
static FUTEX_WAITERS: spin::Mutex<[(usize, usize); 256]> = spin::Mutex::new([(0, 0); 256]);

const ETIMEDOUT: c_int = 110;
const CLOCK_REALTIME: clockid_t = 0;
const CLOCK_MONOTONIC_RAW: clockid_t = 4;
const CLOCK_REALTIME_COARSE: clockid_t = 5;
const CLOCK_MONOTONIC_COARSE: clockid_t = 6;
const CLOCK_BOOTTIME: clockid_t = 7;

const NSEC_PER_SEC: u64 = 1_000_000_000;

/// EPCM permissions of committed heap pages. Pages stay committed for the
/// life of the keep and are reused for any mapping, so they are accepted as
/// RWX once; the ledger keeps track of the protection each mapping asked for.
const RWX: Flags =
    Flags::from_bits_truncate(Flags::READ.bits() | Flags::WRITE.bits() | Flags::EXECUTE.bits());

fn timespec_ns(t: &timespec) -> u64 {
    (t.tv_sec as u64)
        .saturating_mul(NSEC_PER_SEC)
        .saturating_add(t.tv_nsec as u64)
}

fn futex_add_waiter(key: usize) -> sallyport::Result<()> {
    let mut table = FUTEX_WAITERS.lock();
    if let Some(slot) = table.iter_mut().find(|(k, _)| *k == key) {
        slot.1 += 1;
        return Ok(());
    }
    let slot = table.iter_mut().find(|(k, _)| *k == 0).ok_or(ENOMEM)?;
    *slot = (key, 1);
    Ok(())
}

fn futex_remove_waiter(key: usize) {
    let mut table = FUTEX_WAITERS.lock();
    if let Some(slot) = table.iter_mut().find(|(k, _)| *k == key) {
        slot.1 -= 1;
        if slot.1 == 0 {
            slot.0 = 0;
        }
    }
}

fn futex_waiter_count(key: usize) -> usize {
    FUTEX_WAITERS
        .lock()
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(0, |(_, n)| *n)
}

// For `Handler::mmap_guest()`
static ZERO: Page = Page::zeroed();

fn is_prot_allowed(prot: c_int) -> bool {
    prot == PROT_READ || prot == (PROT_READ | PROT_WRITE) || prot == (PROT_READ | PROT_EXEC)
}

fn flags_from_libc(prot: c_int) -> Flags {
    let mut flags = Flags::empty();

    if prot & PROT_READ != 0 {
        flags |= Flags::READ;
    }

    if prot & PROT_WRITE != 0 {
        flags |= Flags::WRITE;
    }

    if prot & PROT_EXEC != 0 {
        flags |= Flags::EXECUTE;
    }

    flags
}

fn access_from_libc(prot: c_int) -> Access {
    let mut access = Access::empty();

    if prot & PROT_READ != 0 {
        access |= Access::READ;
    }

    if prot & PROT_WRITE != 0 {
        access |= Access::WRITE;
    }

    if prot & PROT_EXEC != 0 {
        access |= Access::EXECUTE;
    }

    access
}

trait IntoNonNull {
    fn into_nonnull<T>(self) -> NonNull<T>;
}

impl<R, S> IntoNonNull for Address<R, S>
where
    Address<R, S>: Into<Address<usize, S>>,
{
    fn into_nonnull<T>(self) -> NonNull<T> {
        NonNull::new(self.as_mut_ptr() as *mut T).unwrap()
    }
}

#[derive(PartialEq)]
enum MMapStrategy {
    /// Memory should be mmap'ed directly
    Direct,
    /// Memory should be mmap'ed, when used
    Lazy,
}

/// Thread local storage for the current thread
pub struct Handler<'a> {
    block: &'a mut [usize],
    ssa: &'a mut StateSaveArea,
    tcb: &'a mut Tcb,
    start: u64,
}

impl<'a> Write for Handler<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let buf = s.as_bytes();
        let len = buf.len();
        let mut written = 0;
        while written < len {
            //debugln!(self, "write tempest kill {:?}", written);
            written += self
                .write_host(STDERR_FILENO, &buf[written..])
                .map_err(|_| core::fmt::Error)?;
        }
        Ok(())
    }
}

impl guest::Handler for Handler<'_> {
    fn sally(&mut self) -> sallyport::Result<()> {
        // prevent earlier writes from being moved beyond this point
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);

        unsafe {
            // Safety: Enclave exit and re-enter should have left all registers intact.
            asm!("syscall");
        }

        // prevent later reads from being moved before this point
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);

        Ok(())
    }

    fn debug_futex_wake(
        &mut self,
        uaddr: *const AtomicU32,
        futex_op: c_int,
        val: u32,
        val3: u32,
        current: u32,
        waiters: usize,
        wake_count: usize,
    ) {
        let tid = self.tcb.tid;
        debugln!(
            self,
            "[{tid}] futex wake: uaddr={uaddr:p} op={futex_op:#x} val={val} val3={val3:#x} current={current} waiters={waiters} wake_count={wake_count}",
        );
    }

    fn block(&self) -> &[usize] {
        self.block
    }

    fn block_mut(&mut self) -> &mut [usize] {
        self.block
    }

    fn thread_local_storage(&mut self) -> &mut ThreadLocalStorage {
        &mut self.tcb.tls
    }

    fn arch_prctl(
        &mut self,
        _platform: &impl Platform,
        _code: c_int,
        _addr: c_ulong,
    ) -> sallyport::Result<()> {
        let tid = self.tcb.tid;
        debugln!(self, "[{tid}] arch_prctl should have never been called");
        Err(ENOSYS)
    }

    fn brk(
        &mut self,
        _platform: &impl Platform,
        addr: Option<NonNull<c_void>>,
    ) -> sallyport::Result<NonNull<c_void>> {
        let addr = Address::<usize, Page>::new(
            addr.map(|v| (v.as_ptr() as usize + Page::SIZE - 1) & !(Page::SIZE - 1))
                .unwrap_or(0),
        );

        let mut heap = HEAP.write();
        let max = heap.brk_max();
        let addr = heap.brk(addr);

        if addr > max {
            self.map_heap_on_host(&heap);
            self.mmap_guest(max, addr - max, RWX);
        }

        Ok(addr.into_nonnull())
    }

    fn clone(
        &mut self,
        flags: CloneFlags,
        stack: NonNull<c_void>,
        ptid: Option<&AtomicU32>,
        clear_on_exit: Option<&AtomicU32>,
        tls: NonNull<c_void>,
    ) -> sallyport::Result<c_int> {
        let tid = self.tcb.tid;

        if flags
            != CloneFlags::VM
                | CloneFlags::FS
                | CloneFlags::FILES
                | CloneFlags::SIGHAND
                | CloneFlags::THREAD
                | CloneFlags::SYSVSEM
                | CloneFlags::SETTLS
                | CloneFlags::PARENT_SETTID
                | CloneFlags::CHILD_CLEARTID
                | CloneFlags::DETACHED
        {
            return Err(ENOTSUP);
        }

        let clear_on_exit = clear_on_exit.ok_or(EINVAL)?;
        let ptid = ptid.ok_or(EINVAL)?;

        debugln!(
            self,
            "[{tid}] clone({flags:?}, stack = {stack:p}, ptid = {ptid:p}, clear_on_exit = {clear_on_exit:p}, tls = {tls:p})",
            ptid = ptid as *const _,
            clear_on_exit = clear_on_exit as *const _,
            stack = stack.as_ptr(),
            tls = tls.as_ptr()
        );

        if !POOL_STARTED.swap(true, Ordering::SeqCst) {
            self.start_thread_pool();
        }

        let new_tid = THREAD_ID_CNT.fetch_add(1, Ordering::SeqCst);

        let mut regs = self.ssa.gpr;
        regs.rsp = stack.as_ptr() as _;
        regs.fsbase = tls.as_ptr() as _;

        let new_thread = NewThread::Thread(NewThreadFromRegisters {
            clear_on_exit: clear_on_exit as *const _ as _,
            regs,
            tid: new_tid,
        });

        // A thread is waiting in the enclave: give it the new thread instead
        // of asking the host for one.
        if take_idle() {
            ptid.store(new_tid as _, Ordering::Relaxed);
            NEW_THREAD_QUEUE.write().push(new_thread).unwrap();
            return Ok(new_tid);
        }

        let mut threads_free_guard = THREADS_FREE.write();

        let addr = if *threads_free_guard == 0 {
            debugln!(self, "[{tid}] allocating new thread");
            self.thread_mem_alloc().map_err(|_| EAGAIN)? as usize
        } else {
            *threads_free_guard -= 1;
            0
        };

        drop(threads_free_guard);

        ptid.store(new_tid as _, Ordering::Relaxed);

        NEW_THREAD_QUEUE.write().push(new_thread).unwrap();

        let ret = self.spawn(addr);
        debugln!(self, "[{tid}] spawn() = {ret:#?}");
        ret?;

        Ok(new_tid)
    }

    fn exit(&mut self, status: c_int) -> sallyport::Result<()> {
        let tid = self.tcb.tid;
        let addr = self.tcb.clear_on_exit;
        if let Some(addr) = addr {
            debugln!(self, "[{tid}] clear TID at {addr:p}");
            unsafe { (*addr.as_ptr()).store(0, Ordering::SeqCst) };
            let _ = self.unpark();
        } else {
            debugln!(self, "[{tid}] no TID to clear");
        }

        if self.tcb.return_to_main.rip != 0 {
            self.ssa.gpr.rsp = self.tcb.return_to_main.rsp;
            // The syscall handler will add 2 to rip to skip the syscall instruction (which we don't have anymore)
            self.ssa.gpr.rip = self.tcb.return_to_main.rip - 2;
            self.ssa.gpr.rbx = self.tcb.return_to_main.rbx;
            self.ssa.gpr.rbp = self.tcb.return_to_main.rbp;
            self.ssa.gpr.gsbase = self.tcb.return_to_main.gsbase;
            self.ssa.gpr.fsbase = self.tcb.return_to_main.fsbase;
            self.ssa.gpr.rcx = status as _;

            debugln!(self, "[{tid}] exiting with status {status}",);
        } else {
            debugln!(self, "return_to_main.rip == 0");
            self.print_ssa_stack_trace();
            self.attacked()
        }
        Ok(())
    }

    fn madvise(
        &mut self,
        _platform: &impl Platform,
        _addr: NonNull<c_void>,
        _length: c_size_t,
        _advice: c_int,
    ) -> sallyport::Result<()> {
        Ok(())
    }

    fn mmap(
        &mut self,
        _platform: &impl Platform,
        addr: Option<NonNull<c_void>>,
        len: c_size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: off_t,
    ) -> sallyport::Result<NonNull<c_void>> {
        debugln!(
            self,
            "[enarx-mmap] addr={:?} len={:#x} prot={:#x} flags={:#x} fd={} offset={}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        let addr = addr.map(|v| v.as_ptr() as usize).unwrap_or(0);
        // TODO: https://github.com/enarx/enarx/issues/1892
        let prot = prot | PROT_READ;

        if addr != 0 || len == 0 || fd != -1 || offset != 0 || flags != MAP_PRIVATE | MAP_ANONYMOUS
        {
            return Err(ENOTSUP);
        }

        if prot != 0 && !is_prot_allowed(prot) {
            return Err(EACCES);
        }

        self.do_mmap(prot, len, MMapStrategy::Lazy)
    }

    fn mprotect(
        &mut self,
        _platform: &impl Platform,
        addr: NonNull<c_void>,
        len: c_size_t,
        prot: c_int,
    ) -> sallyport::Result<()> {
        let mut heap = HEAP.write();

        self.mprotect_unlocked(&mut heap, addr, len, prot)
    }

    fn munmap(
        &mut self,
        _platform: &impl Platform,
        addr: NonNull<c_void>,
        length: c_size_t,
    ) -> sallyport::Result<()> {
        let mut heap = HEAP.write();

        self.munmap_unlocked(&mut heap, addr, length)
    }

    fn write(&mut self, fd: c_int, buf: &[u8]) -> sallyport::Result<c_size_t> {
        // Console output stays in the enclave until exit.
        if fd == STDOUT_FILENO || fd == STDERR_FILENO {
            if CONSOLE.lock().push(fd as u8, buf) {
                return Ok(buf.len());
            }
            self.flush_console();
            if CONSOLE.lock().push(fd as u8, buf) {
                return Ok(buf.len());
            }
        }
        self.write_host(fd, buf)
    }

    fn exit_group(&mut self, status: c_int) -> sallyport::Result<()> {
        self.flush_console();
        self.execute(sys::ExitGroup { status })??;
        self.attacked()
    }

    fn clock_gettime(&mut self, clockid: clockid_t, tp: &mut timespec) -> sallyport::Result<()> {
        let realtime = match clockid {
            CLOCK_REALTIME | CLOCK_REALTIME_COARSE => true,
            CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME => {
                false
            }
            _ => return self.execute(ClockGettime { clockid, tp })?,
        };
        let ns = self.clock_ns(realtime);
        tp.tv_sec = (ns / NSEC_PER_SEC) as _;
        tp.tv_nsec = (ns % NSEC_PER_SEC) as _;
        Ok(())
    }

    fn futex(
        &mut self,
        uaddr: &mut AtomicU32,
        futex_op: c_int,
        val: u32,
        timespec: Option<&timespec>,
        _uaddr2: Option<&mut AtomicU32>,
        val3: u32,
    ) -> sallyport::Result<c_long> {
        let realtime = futex_op & FUTEX_CLOCK_REALTIME != 0;
        let op = futex_op & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);
        let key = uaddr as *const AtomicU32 as usize;

        match op {
            FUTEX_WAIT | FUTEX_WAIT_BITSET => {
                if op == FUTEX_WAIT_BITSET && val3 != FUTEX_BITSET_MATCH_ANY {
                    return Err(ENOTSUP);
                }

                // Deadline on the monotonic clock. FUTEX_WAIT takes a relative
                // timeout, FUTEX_WAIT_BITSET an absolute one.
                let deadline = match timespec {
                    None => None,
                    Some(t) if op == FUTEX_WAIT => {
                        Some(self.clock_ns(false).saturating_add(timespec_ns(t)))
                    }
                    Some(t) if realtime => {
                        let offset = self.clock_ns(true) - self.clock_ns(false);
                        Some(timespec_ns(t).saturating_sub(offset))
                    }
                    Some(t) => Some(timespec_ns(t)),
                };

                // Read the epoch before checking the value: a wake that
                // follows a change of the value then always changes it.
                let epoch = FUTEX_EPOCH.load(Ordering::SeqCst);
                if uaddr.load(Ordering::SeqCst) != val {
                    return Err(EAGAIN);
                }

                futex_add_waiter(key)?;
                let mut spins = 1;
                let ret = loop {
                    if uaddr.load(Ordering::SeqCst) != val
                        || FUTEX_EPOCH.load(Ordering::SeqCst) != epoch
                    {
                        break Ok(0);
                    }
                    if let Some(deadline) = deadline {
                        if clock::now_ns(false) >= deadline {
                            break Err(ETIMEDOUT);
                        }
                    }
                    spins = backoff(spins);
                };
                futex_remove_waiter(key);
                ret
            }
            FUTEX_WAKE | FUTEX_WAKE_BITSET => {
                if op == FUTEX_WAKE_BITSET && val3 == 0 {
                    return Ok(0);
                }
                FUTEX_EPOCH.fetch_add(1, Ordering::SeqCst);
                Ok(core::cmp::min(futex_waiter_count(key), val as usize) as c_long)
            }
            _ => Err(ENOTSUP),
        }
    }

    fn park(
        &mut self,
        expected_val: c_int,
        timeout: Option<&timespec>,
    ) -> sallyport::Result<c_int> {
        let deadline = timeout.map(timespec_ns);
        let mut spins = 1;
        loop {
            let epoch = FUTEX_EPOCH.load(Ordering::SeqCst) as c_int;
            if epoch != expected_val {
                return Ok(epoch);
            }
            if let Some(deadline) = deadline {
                if self.clock_ns(false) >= deadline {
                    return Err(ETIMEDOUT);
                }
            }
            spins = backoff(spins);
        }
    }

    fn unpark(&mut self) -> sallyport::Result<()> {
        FUTEX_EPOCH.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn sched_yield(&mut self) -> sallyport::Result<()> {
        core::hint::spin_loop();
        Ok(())
    }

    fn set_tid_address(&mut self, tidptr: &mut c_int) -> sallyport::Result<pid_t> {
        let tid = self.tcb.tid;
        debugln!(
            self,
            "[{tid}] set_tid_address at {tidptr:p}",
            tidptr = tidptr as *const _
        );

        self.tcb.clear_on_exit = NonNull::new(tidptr as *mut c_int as *mut AtomicU32);
        Ok(tid)
    }
}

impl<'a> Handler<'a> {
    fn new(
        ssa: &'a mut StateSaveArea,
        block: &'a mut [usize],
        tcb: &'a mut Tcb,
        start: u64,
    ) -> Self {
        Self {
            ssa,
            block,
            tcb,
            start,
        }
    }

    /// Finish handling an exception
    pub fn finish(ssa: &'a mut StateSaveArea, block: Option<&'a mut [usize]>, tcb: &'a mut Tcb) {
        match ssa.vector() {
            Some(Vector::InvalidOpcode) => {
                if let OP_SYSCALL | OP_CPUID = unsafe { read_unaligned(ssa.gpr.rip as _) } {
                    // Skip the instruction.
                    ssa.gpr.rip += 2;
                    return;
                }
                // Some other invalid opcode.
                panic!();
            }
            Some(Vector::Page) => {
                if let Some(block) = block {
                    let mut h = Self::new(ssa, block, tcb, 0);
                    let error_code =
                        PageFaultErrorCode::from_bits_truncate(h.ssa.misc.exinfo.errcd as u64);
                    let addr = h.ssa.misc.exinfo.maddr as usize;
                    if h.handle_page_fault(addr, error_code).is_ok() {
                        return;
                    }
                    {
                        use core::fmt::Write;
                        let rip = h.ssa.gpr.rip;
                        let _ = writeln!(
                            h,
                            "[shim] UNHANDLED page fault addr {addr:#x} errcd {error_code:#?} rip {rip:#x} -> panic"
                        );
                    }
                }
                // Page fault cannot be handled by the enclave without a block.
                panic!();
            }
            _ => panic!(),
        }
    }

    /// Handle an exception
    pub fn handle(
        ssa: &'a mut StateSaveArea,
        block: &'a mut [usize],
        tcb: &'a mut Tcb,
        start: u64,
    ) {
        let mut h = Self::new(ssa, block, tcb, start);

        match h.ssa.vector() {
            Some(Vector::InvalidOpcode) => match unsafe { read_unaligned(h.ssa.gpr.rip as _) } {
                OP_SYSCALL => h.handle_syscall(),
                OP_CPUID => h.handle_cpuid(),
                r => {
                    debugln!(h, "unsupported opcode: {:#04x}", r);
                    h.print_ssa_stack_trace();

                    #[cfg(feature = "dbg")]
                    if r as u8 == 0xCC {
                        h.ssa.gpr.rip += 1;
                        return;
                    }

                    #[cfg(feature = "gdb")]
                    if r as u8 == 0xCC {
                        let rip = h.ssa.gpr.rip;
                        if unsafe { crate::handler::gdb::unset_bp(rip) } {
                            debugln!(h, "unset_bp: {:#x}", rip);
                        }
                    }

                    #[cfg(feature = "gdb")]
                    h.gdb_session();

                    if r == unsafe { read_unaligned(h.ssa.gpr.rip as _) } {
                        let _ = h.exit_group(1);
                        unreachable!()
                    }
                }
            },

            Some(Vector::Page) => {
                let error_code =
                    PageFaultErrorCode::from_bits_truncate(h.ssa.misc.exinfo.errcd as u64);
                if h.handle_page_fault(h.ssa.misc.exinfo.maddr as usize, error_code)
                    .is_ok()
                {
                    return;
                }

                if cfg!(feature = "gdb") {
                    h.print_ssa_stack_trace();
                    #[cfg(feature = "gdb")]
                    h.gdb_session();
                    let _ = h.exit_group(1);
                } else {
                    h.attacked()
                }
            }

            _ => {
                debugln!(h, "unhandled exception: {:#?}", h.ssa.vector());
                debugln!(h, "Exinfo: {:#?}", h.ssa.misc.exinfo.clone());

                if cfg!(feature = "dbg") {
                    h.print_ssa_stack_trace();
                }
                h.attacked()
            }
        }
    }

    fn get_key(
        &mut self,
        platform: &impl Platform,
        buf: usize,
        buf_len: usize,
    ) -> Result<usize, c_int> {
        if buf == 0 {
            return Ok(key::SGX_KEY_LEN);
        }

        if buf_len > isize::MAX as usize {
            return Err(EINVAL);
        }

        if buf_len < key::SGX_KEY_LEN {
            return Err(EMSGSIZE);
        }

        let buf = platform.validate_slice_mut::<u8>(buf, buf_len)?;

        let key_request = key::Request {
            name: key::Names::SealKey,
            policy: key::Policy::MRSIGNER,
            isvsvn: 0,
            ..Default::default()
        };

        let key_response = key_request.enclu_egetkey().map_err(|e| {
            debugln!(self, "enclu_egetkey: {}", e);
            EIO
        })?;

        buf[..key::SGX_KEY_LEN].copy_from_slice(&key_response.key);

        Ok(key::SGX_KEY_LEN)
    }

    fn get_attestation(
        &mut self,
        platform: &impl Platform,
        hash: usize,
        hash_len: usize,
        buf: usize,
        buf_len: usize,
    ) -> Result<[usize; 2], c_int> {
        if cfg!(feature = "disable-sgx-attestation") {
            return Ok([0, 0]);
        }

        let mut target_info = TargetInfo::default();

        self.get_sgx_target_info(&mut target_info)?;

        let quote_size = self.get_sgx_quote_size()?;

        if buf == 0 {
            return Ok([quote_size, TECH]);
        }

        if buf_len > isize::MAX as usize {
            return Err(EINVAL);
        }

        if buf_len < quote_size {
            return Err(EMSGSIZE);
        }

        if hash_len != 64 {
            return Err(EINVAL);
        }

        let hash = {
            let h = platform.validate_slice::<u8>(hash, hash_len)?;
            let mut hash = [0u8; 64];
            hash.copy_from_slice(h);
            hash
        };

        let buf = platform.validate_slice_mut::<u8>(buf, buf_len)?;

        // Generate Report
        let report: Report = target_info.enclu_ereport(&ReportData(hash));

        let len = self.get_sgx_quote(&report, buf)?;

        Ok([len, TECH])
    }

    /// Write to a host file descriptor, bypassing the console buffer.
    fn write_host(&mut self, fd: c_int, buf: &[u8]) -> sallyport::Result<c_size_t> {
        self.execute(sys::Write { fd, buf })?
            .unwrap_or_else(|| self.attacked())
    }

    /// Write the buffered console output to the host.
    fn flush_console(&mut self) {
        let mut console = CONSOLE.lock();
        console.drain(|fd, mut data| {
            while !data.is_empty() {
                match self.write_host(fd as c_int, data) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => data = &data[n..],
                }
            }
        });
    }

    /// Nanoseconds of the real-time or monotonic clock, from the TSC.
    fn clock_ns(&mut self, realtime: bool) -> u64 {
        if !clock::ready() {
            self.clock_init();
        }
        clock::now_ns(realtime)
    }

    /// Anchor the TSC clock to the host clock. This is the only time the
    /// clock asks the host; it happens on the first clock read or when the
    /// thread pool starts, during keep startup.
    fn clock_init(&mut self) {
        let host = |h: &mut Self, clockid| {
            let mut t = timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            h.execute(ClockGettime {
                clockid,
                tp: &mut t,
            })
            .and_then(|r| r)
            .unwrap_or_else(|_| h.attacked());
            timespec_ns(&t)
        };

        // TSC frequency: CPUID leaf 0x15 if it reports one, otherwise
        // measured against the host's monotonic clock over ~20 ms.
        let mut leaf = CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        };
        let hz = if self.cpuid(0x15, 0, &mut leaf).is_ok()
            && leaf.eax != 0
            && leaf.ebx != 0
            && leaf.ecx != 0
        {
            leaf.ecx as u64 * leaf.ebx as u64 / leaf.eax as u64
        } else {
            let t0 = host(self, CLOCK_MONOTONIC);
            let c0 = clock::rdtsc();
            while clock::rdtsc() - c0 < 50_000_000 {
                core::hint::spin_loop();
            }
            let t1 = host(self, CLOCK_MONOTONIC);
            let c1 = clock::rdtsc();
            ((c1 - c0) as u128 * NSEC_PER_SEC as u128 / (t1 - t0) as u128) as u64
        };

        let monotonic = host(self, CLOCK_MONOTONIC);
        let realtime = host(self, CLOCK_REALTIME);
        clock::init(hz, clock::rdtsc(), monotonic, realtime);
        debugln!(self, "clock: TSC {hz} Hz");
    }

    /// Start the threads that wait in the enclave for `clone()`, and anchor
    /// the clock, while the keep is still starting up.
    fn start_thread_pool(&mut self) {
        if !clock::ready() {
            self.clock_init();
        }
        for _ in 0..POOL_THREADS {
            let tcs = match self.thread_mem_alloc() {
                Ok(tcs) => tcs as usize,
                Err(_) => break,
            };
            POOL_PENDING.fetch_add(1, Ordering::SeqCst);
            if self.spawn(tcs).is_err() {
                POOL_PENDING.fetch_sub(1, Ordering::SeqCst);
                break;
            }
        }
    }

    fn handle_syscall(&mut self) {
        let orig_rdx = self.ssa.gpr.rdx;
        let nr = self.ssa.gpr.rax as usize;
        let tid = self.tcb.tid;
        let futex_args = if nr == SYS_futex as usize {
            Some((
                self.ssa.gpr.rdi,
                self.ssa.gpr.rsi,
                self.ssa.gpr.rdx,
                self.ssa.gpr.r10,
                self.ssa.gpr.r8,
                self.ssa.gpr.r9,
            ))
        } else {
            None
        };

        // reduce log spam
        if nr != SYS_clock_gettime as _ && nr != SYS_sched_yield as _ {
            debugln!(self, "[{tid}] syscall {nr} ...");
        }
        if let Some((uaddr, op, val, timeout, uaddr2, val3)) = futex_args {
            debugln!(
                self,
                "[{tid}] futex enter: uaddr={:#x} op={:#x} val={} timeout={:#x} uaddr2={:#x} val3={:#x}",
                uaddr,
                op,
                val,
                timeout,
                uaddr2,
                val3
            );
        }

        let usermemscope = UserMemScope;

        match nr as i64 {
            SYS_GETKEY => {
                let ret = self.get_key(&usermemscope, self.ssa.gpr.rdi as _, self.ssa.gpr.rsi as _);
                match ret {
                    Err(e) => self.ssa.gpr.rax = -e as u64,
                    Ok(rax) => {
                        self.ssa.gpr.rax = rax as u64;
                        self.ssa.gpr.rdx = orig_rdx;
                    }
                }
            }
            SYS_GETATT => {
                let ret = self.get_attestation(
                    &usermemscope,
                    self.ssa.gpr.rdi as _,
                    self.ssa.gpr.rsi as _,
                    self.ssa.gpr.rdx as _,
                    self.ssa.gpr.r10 as _,
                );
                match ret {
                    Err(e) => self.ssa.gpr.rax = -e as u64,
                    Ok([rax, rdx]) => {
                        self.ssa.gpr.rax = rax as u64;
                        self.ssa.gpr.rdx = rdx as u64;
                    }
                }
            }
            _ => unsafe {
                // Safety:
                // with `usermemscope` we
                // * limit the lifetime of objects created from the userspace syscall arguments to this function.
                // * make sure only memory of the userspace application is addressed
                let ret = self.syscall(
                    &usermemscope,
                    [
                        nr,
                        self.ssa.gpr.rdi as usize,
                        self.ssa.gpr.rsi as usize,
                        self.ssa.gpr.rdx as usize,
                        self.ssa.gpr.r10 as usize,
                        self.ssa.gpr.r8 as usize,
                        self.ssa.gpr.r9 as usize,
                    ],
                );
                match ret {
                    Err(e) => self.ssa.gpr.rax = -e as u64,
                    Ok([rax, _]) => {
                        self.ssa.gpr.rax = rax as u64;
                        self.ssa.gpr.rdx = orig_rdx;
                    }
                }
            },
        };

        self.ssa.gpr.rip += 2;

        // reduce log spam
        if nr != SYS_clock_gettime as _ && nr != SYS_sched_yield as _ {
            debugln!(self, "[{tid}] syscall {nr} = {}", self.ssa.gpr.rax as isize);
        }
        if let Some((uaddr, op, val, timeout, uaddr2, val3)) = futex_args {
            debugln!(
                self,
                "[{tid}] futex exit: uaddr={:#x} op={:#x} val={} timeout={:#x} uaddr2={:#x} val3={:#x} ret={}",
                uaddr,
                op,
                val,
                timeout,
                uaddr2,
                val3,
                self.ssa.gpr.rax as isize
            );
        }
    }

    fn handle_cpuid(&mut self) {
        let mut cpuid_result: CpuidResult = CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        };

        debug!(
            self,
            "cpuid({:08x}, {:08x})",
            self.ssa.gpr.rax.clone(),
            self.ssa.gpr.rcx.clone(),
        );

        self.cpuid(
            self.ssa.gpr.rax as _,
            self.ssa.gpr.rcx as _,
            &mut cpuid_result,
        )
        .unwrap();

        self.ssa.gpr.rax = cpuid_result.eax.into();
        self.ssa.gpr.rbx = cpuid_result.ebx.into();
        self.ssa.gpr.rcx = cpuid_result.ecx.into();
        self.ssa.gpr.rdx = cpuid_result.edx.into();

        debugln!(
            self,
            " = ({:08x}, {:08x}, {:08x}, {:08x})",
            self.ssa.gpr.rax.clone(),
            self.ssa.gpr.rbx.clone(),
            self.ssa.gpr.rcx.clone(),
            self.ssa.gpr.rdx.clone()
        );

        self.ssa.gpr.rip += 2;
    }

    fn do_mmap(
        &mut self,
        prot: c_int,
        len: usize,
        lazy: MMapStrategy,
    ) -> Result<NonNull<c_void>, Error> {
        if len == 0 {
            return Err(EINVAL);
        }

        let length = Offset::from_items((len + Page::SIZE - 1) / Page::SIZE);
        let access = access_from_libc(prot);
        let mut heap = HEAP.write();

        let Some(addr) = heap.mmap(None, length, access) else {
            debugln!(self, "ERROR heap.mmap() failed!!!!");
            return Err(ENOMEM);
        };

        let ret = addr.into_nonnull();
        debugln!(
            self,
            "mmap({:#?})",
            Record {
                region: Region::new(addr, addr + length),
                access: access | Access::MMAPPED
            }
        );

        if lazy == MMapStrategy::Lazy {
            // Allocate all paged on-demand.
            return Ok(ret);
        }

        if heap.mmap(Some(addr), length, access | Access::MMAPPED) != Some(addr) {
            debugln!(self, "MMAPPED heap is inconsistent");
            self.attacked()
        }

        self.map_heap_on_host(&heap);
        self.mmap_guest(addr, length, flags_from_libc(prot));

        // If the previous operations succeeded, the virtual memory area
        // (VMA) is already RW.
        if prot != PROT_READ | PROT_WRITE {
            if let Err(e) = self.mprotect_unlocked(&mut heap, ret, length.bytes() as c_size_t, prot)
            {
                debugln!(self, "ERROR mprotect_unlocked() = {e:#?}");
                self.attacked()
            }
        }
        Ok(ret)
    }

    /// Commit pages: accept the pages the host adds with ENCLS[EAUG] on first
    /// touch. Pages that are already committed are left alone; `munmap()`
    /// zeroed them when they were last unmapped.
    fn mmap_guest(
        &mut self,
        addr: Address<usize, Page>,
        length: Offset<usize, Page>,
        flags: Flags,
    ) {
        let zero_virt_addr = VirtAddr::new(ZERO.as_ptr() as u64);
        // # Safety
        //
        // The address must be page aligned.
        let zero_page_addr = unsafe { PageAddr::from_start_address_unchecked(zero_virt_addr) };

        for i in 0..length.items() {
            let raw = addr.raw() + i * Page::SIZE;
            if committed::contains(raw) {
                continue;
            }
            let virt_addr = VirtAddr::new(raw as u64);
            // # Safety
            //
            // The address must be page aligned.
            let page_addr = unsafe { PageAddr::from_start_address_unchecked(virt_addr) };

            Class::Regular
                .info(flags)
                .accept_copy(page_addr, zero_page_addr)
                .unwrap_or_else(|_| self.attacked());
            committed::insert(raw);
        }
    }

    /// Map the whole heap on the host, once, so that committing a page
    /// later needs no host call.
    fn map_heap_on_host(&mut self, heap: &Heap) {
        if HEAP_HOST_MAPPED.load(Ordering::Acquire) {
            return;
        }
        let span = heap.span();
        if let Err(e) = self.mmap_host(
            span.start.into_nonnull(),
            (span.end - span.start).bytes(),
            PROT_READ | PROT_WRITE | PROT_EXEC,
        ) {
            debugln!(self, "ERROR mmap_host(heap) = {e:#?}");
            self.attacked()
        }
        HEAP_HOST_MAPPED.store(true, Ordering::Release);
    }

    fn mprotect_unlocked(
        &mut self,
        heap: &mut Heap,
        addr_in: NonNull<c_void>,
        length_in: c_size_t,
        prot: c_int,
    ) -> sallyport::Result<()> {
        let addr = addr_in.as_ptr() as usize;
        // TODO: Simplify:
        let pages = ((length_in + Page::SIZE - 1) & !(Page::SIZE - 1)) / Page::SIZE;

        if addr & 0xfff != 0 || pages == 0 {
            return Err(EINVAL);
        }

        if !is_prot_allowed(prot) {
            return Err(EACCES);
        }

        let addr = Address::new(addr);
        let length = Offset::from_items(pages);
        let access = access_from_libc(prot);

        debugln!(self, "heap = {heap:#?}", heap = heap.deref());
        debugln!(
            self,
            "mprotect({:#?}, {:?})",
            &Region {
                start: addr,
                end: addr + length
            },
            access
        );

        match heap.contains(addr, length) {
            None => return Err(ENOMEM),
            Some(access) => {
                if access == Access::empty() {
                    debugln!(
                        self,
                        "mprotect_unlocked previous access ({:#?}",
                        Record {
                            region: Region::new(addr, addr + length),
                            access
                        }
                    );
                    self.attacked();
                }
            }
        }

        // Committed pages are RWX in the EPCM and on the host, so a change of
        // protection is recorded in the ledger only.
        heap.protect_with(addr, length, |record| {
            debugln!(self, "mprotect_unlocked working on ({record:#?})");
            access | (record.access & Access::MMAPPED)
        })
        .unwrap_or_else(|e| {
            debugln!(self, "mprotect_unlocked heap.protect_with failed: {:?}", e);
            self.attacked();
        });

        Ok(())
    }

    fn munmap_unlocked(
        &mut self,
        heap: &mut Heap,
        addr_in: NonNull<c_void>,
        length_in: c_size_t,
    ) -> sallyport::Result<()> {
        let tid = self.tcb.tid;
        let addr = addr_in.as_ptr() as usize;
        let pages = ((length_in + Page::SIZE - 1) & !(Page::SIZE - 1)) / Page::SIZE;

        if addr & 0xfff != 0 || pages == 0 {
            return Err(EINVAL);
        }

        let addr = Address::new(addr);
        let length = Offset::from_items(pages);

        if heap.contains(addr, length).is_none() {
            return Ok(());
        }

        // Committed pages are not given back to the host: zero them, so that
        // the next mapping of the address starts clean, and keep them.
        heap.munmap_with(addr, length, move |record: &Record<Access>| {
            debugln!(self, "[{tid}] sgx_unmap({:#?}", record);

            committed::for_each(record.region.start.raw(), record.region.end.raw(), |page| {
                // Safety: the page is committed and belongs to the mapping
                // being removed.
                unsafe { core::ptr::write_bytes(page as *mut u8, 0, Page::SIZE) };
            });
        })
        .map_err(|_| ENOMEM)?;

        Ok(())
    }

    /// Unmap pages and give them back to the host (trim), so that touching
    /// them faults. Used for the stack guard pages of new threads, which are
    /// set up when the thread pool starts.
    fn trim_unlocked(
        &mut self,
        heap: &mut Heap,
        addr_in: NonNull<c_void>,
        length_in: c_size_t,
    ) -> sallyport::Result<()> {
        let tid = self.tcb.tid;
        let addr = addr_in.as_ptr() as usize;
        let pages = ((length_in + Page::SIZE - 1) & !(Page::SIZE - 1)) / Page::SIZE;

        if addr & 0xfff != 0 || pages == 0 {
            return Err(EINVAL);
        }

        let addr = Address::new(addr);
        let length = Offset::from_items(pages);

        heap.munmap(addr, length).map_err(|_| ENOMEM)?;

        for i in 0..pages {
            let raw = addr.raw() + i * Page::SIZE;
            if !committed::contains(raw) {
                continue;
            }
            let page = NonNull::new(raw as *mut c_void).unwrap();

            // Failing in any of these operations is expected to crash the
            // enclave because it is due either to a software bug, or a
            // malicious host.
            if let Err(e) = self.modify_sgx_page_type(page, Page::SIZE, Class::Trimmed as _) {
                debugln!(
                    self,
                    "[{tid}] ERROR trim: modify_sgx_page_type FAILED !!! {e:?}"
                );
                self.attacked();
            }

            // # Safety
            //
            // The address is page aligned.
            let page_addr =
                unsafe { PageAddr::from_start_address_unchecked(VirtAddr::new(raw as u64)) };
            Class::Trimmed
                .info(Flags::MODIFIED)
                .accept(page_addr)
                .unwrap_or_else(|_| self.attacked());

            // Remove the page, then map the address on the host again: the
            // heap stays mapped as a whole, so that the address can be
            // committed again later without a host call.
            self.munmap_host(page, Page::SIZE)
                .unwrap_or_else(|_| self.attacked());
            if HEAP_HOST_MAPPED.load(Ordering::Acquire) {
                self.mmap_host(page, Page::SIZE, PROT_READ | PROT_WRITE | PROT_EXEC)
                    .unwrap_or_else(|_| self.attacked());
            }
            committed::remove(raw);
        }

        Ok(())
    }

    /// Print a stack trace using the SSA registers.
    fn print_ssa_stack_trace(&mut self) {
        if DEBUG {
            debugln!(self, "{:#x?}", self.ssa.gpr.clone());
            unsafe { self.print_stack_trace(self.ssa.gpr.rip, self.ssa.gpr.rbp) }
        }
    }

    /// Print out `rip` relative to the shim (S) or the exec (E) base address.
    ///
    /// This can be used with `addr2line` and the executable with debug info
    /// to get the function name and line number.
    unsafe fn print_rip(&mut self, rip: u64) {
        let enarx_exec_start = &ENARX_EXEC_START as *const _ as u64;
        let enarx_exec_end = &ENARX_EXEC_END as *const _ as u64;

        let exec_range = enarx_exec_start..enarx_exec_end;

        if exec_range.contains(&rip) {
            let rip_pie = rip - enarx_exec_start;
            debugln!(self, "E {:>#016x}", rip_pie);
        } else {
            let rip_pie = rip - shim_address() as u64;
            debugln!(self, "S {:>#016x}", rip_pie);
        }
    }

    /// Print a stack trace with the old `rbp` stack frame pointers
    unsafe fn print_stack_trace(&mut self, rip: u64, mut rbp: u64) {
        // TODO: parse the elf and actually find the text sections.
        let encl_start = self as *const _ as u64 / ENCL_SIZE as u64 * ENCL_SIZE as u64;
        let encl_end = encl_start + ENCL_SIZE as u64;
        let encl_range = encl_start..encl_end;

        debugln!(self, "TRACE:");

        self.print_rip(rip);

        // Maximum 64 frames
        for _frame in 0..64 {
            if rbp == 0 || rbp & 7 != 0 {
                break;
            }

            if !encl_range.contains(&rbp) {
                debugln!(self, "invalid rbp: {:>#016x}", rbp);
                break;
            }

            match rbp.checked_add(size_of::<usize>() as _) {
                None => break,
                Some(rip_rbp) => {
                    let rip = *(rip_rbp as *const u64);
                    match rip.checked_sub(1) {
                        None => break,
                        Some(0) => break,
                        Some(rip) => {
                            self.print_rip(rip);
                            rbp = *(rbp as *const u64);
                        }
                    }
                }
            }
        }
    }

    fn thread_mem_alloc(&mut self) -> sallyport::Result<*const Tcs> {
        // Allocate the whole block of memory used for the thread.
        // It is easier to do this in one go and punch holes in it,
        // than to allocate each part separately.
        let addr = self.do_mmap(
            PROT_READ | PROT_WRITE,
            size_of::<ThreadMem>(),
            MMapStrategy::Direct,
        )?;

        // # Safety
        // ThreadMem is a POD type and the memory is aligned and zeroed.
        let tm = unsafe { &mut *(addr.as_ptr() as *mut ThreadMem) };

        let stack_end_1 = NonNull::new(&mut tm.cssa_stack as *mut _ as *mut c_void).unwrap();
        let stack_end_0 = NonNull::new(&mut tm.stack as *mut _ as *mut c_void).unwrap();
        let tcs = &tm.tcs as *const Tcs;
        let ssa = tm.ssa.as_ptr();

        // address is relative to the enclave base (`shim_address()`)
        tm.tcs.ossa = ssa as u64 - shim_address() as u64;
        // start with level 0
        tm.tcs.cssa = 0;
        // number of SSA frames
        tm.tcs.nssa = NUM_SSA as _;
        // address is relative to the enclave base
        tm.tcs.oentry = self.start - shim_address() as u64;

        // unmap stack guard pages, so a stack overflow will cause a page fault
        {
            let mut heap = HEAP.write();
            self.trim_unlocked(&mut heap, stack_end_1, Page::SIZE)
                .unwrap_or_else(|_| self.attacked());
            self.trim_unlocked(&mut heap, stack_end_0, Page::SIZE)
                .unwrap_or_else(|_| self.attacked());
        }

        // The heap is mapped RWX on the host, but the kernel only maps a TCS
        // page into a VMA without execute permission: give the page its
        // own read-write mapping before it becomes a TCS.
        self.mmap_host(
            NonNull::new(tcs as *mut c_void).unwrap(),
            Page::SIZE,
            PROT_READ | PROT_WRITE,
        )
        .unwrap_or_else(|_| self.attacked());

        self.modify_sgx_page_type(
            NonNull::new(tcs as *mut c_void).unwrap(),
            Page::SIZE,
            Class::Tcs as _,
        )
        .unwrap_or_else(|_| self.attacked());

        let virt_addr = VirtAddr::from_ptr(tcs);
        // # Safety
        //
        // The address must be page aligned.
        let page_addr = unsafe { PageAddr::from_start_address_unchecked(virt_addr) };

        Class::Tcs
            .info(Flags::MODIFIED)
            .accept(page_addr)
            .unwrap_or_else(|_| self.attacked());

        Ok(tcs)
    }

    // FIXME: https://github.com/enarx/enarx/issues/2251
    #[allow(dead_code)]
    fn thread_mem_free(&mut self, tcs: *const Tcs) -> sallyport::Result<()> {
        let usermemscope = UserMemScope;

        let tm_start = tcs as usize - CSSA_1_PLUS_STACK_SIZE - CSSA_0_STACK_SIZE - Page::SIZE;
        let tm_addr = NonNull::new(tm_start as *const ThreadMem as *mut c_void).unwrap();

        debugln!(
            self,
            "drop_thread_mem: {:#?} {}",
            tm_addr,
            size_of::<ThreadMem>()
        );

        self.munmap(&usermemscope, tm_addr, size_of::<ThreadMem>())?;

        Ok(())
    }

    fn handle_page_fault(&mut self, addr: usize, error_code: PageFaultErrorCode) -> Result<(), ()> {
        let tid = self.tcb.tid;
        let addr = addr & !(Page::SIZE - 1);

        // debugln!(self, "[{tid}] handle_page_fault: {addr:#x} {error_code:?}");

        // check for non-valid flags
        //
        // PROTECTION_VIOLATION|SGX is accepted here: it is what a thread sees
        // when it touches a page whose EPCM permissions are mid-transition
        // (another thread's mprotect EMODPR/EACCEPT/EMODPE window, or a
        // concurrent first-touch commit). Those faults are resolved further
        // down by serializing on the HEAP lock and retrying.
        if !error_code
            .difference(
                PageFaultErrorCode::CAUSED_BY_WRITE
                    | PageFaultErrorCode::INSTRUCTION_FETCH
                    | PageFaultErrorCode::USER_MODE
                    | PageFaultErrorCode::PROTECTION_VIOLATION
                    | PageFaultErrorCode::SGX,
            )
            .is_empty()
        {
            {
                use core::fmt::Write;
                let _ = writeln!(
                    self,
                    "[shim {tid}] page fault REJECTED addr {addr:#x} difference {:#?}",
                    error_code.difference(
                        PageFaultErrorCode::CAUSED_BY_WRITE
                            | PageFaultErrorCode::INSTRUCTION_FETCH
                            | PageFaultErrorCode::USER_MODE,
                    )
                );
            }

            return Err(());
        }

        // check for must have flags
        if !error_code.contains(PageFaultErrorCode::USER_MODE) {
            return Err(());
        }

        let mut heap = HEAP.write();

        let addr = Address::new(addr);
        let length = Offset::from_items(1);

        let access = match heap.contains(addr, length) {
            None => {
                {
                    use core::fmt::Write;
                    let _ = writeln!(
                        self,
                        "[shim {tid}] page fault: ledger access empty {:#?} errcd {error_code:#?}",
                        Region::new(addr, addr + length),
                    );
                }
                panic!();
            }
            Some(access) => {
                let access_covers_fault = access.contains(Access::READ)
                    && (!error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE)
                        || access.contains(Access::WRITE))
                    && (!error_code.contains(PageFaultErrorCode::INSTRUCTION_FETCH)
                        || access.contains(Access::READ | Access::EXECUTE));

                // check for real page fault
                if !access_covers_fault {
                    {
                        use core::fmt::Write;
                        let _ = writeln!(
                            self,
                            "[shim {tid}] REAL page fault errcd {error_code:#?} previous access {:#?}",
                            Record {
                                region: Region::new(addr, addr + length),
                                access
                            }
                        );
                    }
                    panic!();
                }
                access
            }
        };

        // Another thread committed the page while this one waited for HEAP.
        if committed::contains(addr.raw()) {
            return Ok(());
        }

        // Commit up to 64 pages of the mapping per fault instead of exactly
        // one, stopping at the first page that is already committed.
        let mut length = Offset::from_items(1);
        if let Some((region, _)) = heap.record_at(addr) {
            let max = (region.end - addr).items().min(64);
            let mut pages = 1;
            while pages < max && !committed::contains(addr.raw() + pages * Page::SIZE) {
                pages += 1;
            }
            length = Offset::from_items(pages);
        }

        if heap.mmap(Some(addr), length, access | Access::MMAPPED) != Some(addr) {
            {
                use core::fmt::Write;
                let _ = writeln!(
                    self,
                    "[shim {tid}] page fault: MMAPPED heap is inconsistent at {addr:#?}"
                );
            }
            panic!();
        }

        self.map_heap_on_host(&heap);
        self.mmap_guest(addr, length, RWX);

        Ok(())
    }
}
