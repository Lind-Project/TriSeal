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
    shim_address, BLOCK_SIZE, CSSA_0_STACK_SIZE, CSSA_1_PLUS_STACK_SIZE, DEBUG, ENARX_EXEC_END,
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
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use mmledger::{Record, Region, Span};
use primordial::{Address, Offset, Page};
use sallyport::guest::enarxcall::{CommitShm, MapHostShared, MapShm};
use sallyport::guest::syscall::ClockGettime;
use sallyport::guest::{self, syscall as sys, Handler as _, Platform, ThreadLocalStorage};
use sallyport::item::enarxcall::sgx::{Report, ReportData, TargetInfo, TECH};
use sallyport::item::enarxcall::{
    shared, SYS_CLOCK_EPOCH, SYS_COMMITSHM, SYS_DIRECT_INIT, SYS_GETATT, SYS_GETKEY, SYS_MAPSHM,
    SYS_PHASE, SYS_SALLY,
};
use sallyport::libc::{
    clockid_t, off_t, pid_t, timespec, CloneFlags, SYS_brk, SYS_clock_gettime, SYS_clone, SYS_exit,
    SYS_exit_group, SYS_fcntl, SYS_fstat, SYS_futex, SYS_getegid, SYS_geteuid, SYS_getgid,
    SYS_getpid, SYS_getrandom, SYS_getuid, SYS_ioctl, SYS_madvise, SYS_mmap, SYS_mprotect,
    SYS_mremap, SYS_munmap, SYS_rt_sigprocmask, SYS_sched_yield, SYS_set_tid_address,
    SYS_sigaltstack, SYS_write, SYS_writev, CLOCK_MONOTONIC, EACCES, EAGAIN, EINTR, EINVAL, EIO,
    EMSGSIZE, ENOMEM, ENOSYS, ENOTSUP, EPERM, FUTEX_BITSET_MATCH_ANY, FUTEX_CLOCK_REALTIME,
    FUTEX_PRIVATE_FLAG, FUTEX_WAIT, FUTEX_WAIT_BITSET, FUTEX_WAKE, FUTEX_WAKE_BITSET,
    MAP_ANONYMOUS, MAP_PRIVATE, PROT_EXEC, PROT_READ, PROT_WRITE, STDERR_FILENO, STDOUT_FILENO,
};
use sallyport::Error;
use sgx::page::{Class, Flags};
use sgx::ssa::Vector;
use sgx::ssa::{GenPurposeRegs, StateSaveArea};
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

/// Address of the shim's entry point (`_start`), for new TCSs.
static START: AtomicU64 = AtomicU64::new(0);

/// Record the address of the shim's entry point.
pub fn set_start(start: u64) {
    START.store(start, Ordering::Relaxed);
}

/// The phase page shared with the host (0 until it is mapped).
static PHASE_PAGE: AtomicU64 = AtomicU64::new(0);

/// Size of the phase page and of the console memory shared with the host.
/// They must match what the host maps (`src/backend/sgx/shared.rs`).
const PHASE_SIZE: usize = 4096;
const CONSOLE_SIZE: usize = 64 << 20;

/// Words of the phase page (see `src/backend/sgx/shared.rs`).
const PHASE_WATCH: usize = 0;
const PHASE_MARK: usize = 1;
const PHASE_ACK: usize = 2;
const PHASE_AT_PHASE: usize = 8;
const PHASE_AT_EXIT: usize = 16;

/// Set when the payload marks phase 1 or a later one (`SYS_PHASE`: lind's
/// IMFS grate marks phase 1 when it has staged its files and starts the
/// build), and never cleared. A sealed keep serves only the syscalls that
/// [`sealed_allows`], and asks the host only for what a thread may ask with
/// its [`Tcb::host_permit`] set: console output, the dump commit, a new
/// thread, and exit. Anything else ends the whole run rather than fails: a
/// build step that got an error could take another path, one the host chose.
static SEALED: AtomicBool = AtomicBool::new(false);

/// What the keep exits with when it ends the run rather than let the payload
/// see an error: when it is sealed and asked for something else, when the
/// host does not start a thread the payload asked for, or when the host
/// breaks a rule (128 + SIGSYS, as for a process that seccomp kills).
const FAIL_STOP_EXIT: c_int = 128 + 31;

/// The host file mapped writable with `SYS_MAPSHM` (the dump), or -1.
static DUMP_FD: AtomicI32 = AtomicI32::new(-1);

/// Whether the sealed keep committed the dump: it may do that once.
static DUMP_COMMITTED: AtomicBool = AtomicBool::new(false);

// Syscalls missing in `sallyport::libc`.
const SYS_PRCTL: i64 = 157;
const SYS_GETTID: i64 = 186;
const SYS_TKILL: i64 = 200;
const SYS_PROCESS_VM_WRITEV: i64 = 311;

const CLOCK_PROCESS_CPUTIME_ID: clockid_t = 2;
const CLOCK_THREAD_CPUTIME_ID: clockid_t = 3;

/// Whether a sealed keep serves syscall `nr`, whose first argument is
/// `arg0`. It serves each of them in the enclave, except that console output
/// and the dump commit are sent to the host, and that the request behind
/// `SYS_SALLY` is checked by [`Handler::sally`].
#[allow(non_upper_case_globals)]
fn sealed_allows(nr: i64, arg0: u64) -> bool {
    let stdio = arg0 <= STDERR_FILENO as u64;
    let console = arg0 == STDOUT_FILENO as u64 || arg0 == STDERR_FILENO as u64;
    match nr {
        // Memory.
        SYS_mmap | SYS_munmap | SYS_mprotect | SYS_mremap | SYS_brk | SYS_madvise => true,
        // Synchronization and threads.
        SYS_futex | SYS_clone | SYS_exit | SYS_exit_group | SYS_sched_yield => true,
        // What creating, forking and ending a thread takes besides (lind
        // names its threads, and copies a cage's memory with
        // `process_vm_writev` where it can): the shim keeps the signal mask
        // and stack, and answers the rest with constants or ENOSYS.
        SYS_set_tid_address
        | SYS_rt_sigprocmask
        | SYS_sigaltstack
        | SYS_PRCTL
        | SYS_GETTID
        | SYS_TKILL
        | SYS_getpid
        | SYS_PROCESS_VM_WRITEV => true,
        // The identity the shim reports (constants).
        SYS_getuid | SYS_geteuid | SYS_getgid | SYS_getegid => true,
        // Clocks (from the TSC) and random numbers (RDRAND).
        SYS_clock_gettime | SYS_getrandom => true,
        // Console output, and what a program asks about its standard
        // streams, which the shim answers itself (no tty, a pipe).
        SYS_write | SYS_writev => console,
        SYS_fstat | SYS_fcntl | SYS_ioctl => stdio,
        // The shim's own: phase marks, the dump commit, and a syscall taken
        // through the direct entry that needs the host.
        SYS_PHASE | SYS_COMMITSHM | SYS_SALLY => true,
        _ => false,
    }
}

/// What the shim did, counted in the enclave: the host cannot see the
/// syscalls taken through the direct entry nor the pages committed ahead of
/// use. They are copied to the phase page at each phase mark and at exit.
mod count {
    use core::sync::atomic::AtomicU64;
    pub static DIRECT_SYSCALLS: AtomicU64 = AtomicU64::new(0);
    pub static TRAP_SYSCALLS: AtomicU64 = AtomicU64::new(0);
    pub static PAGE_FAULTS: AtomicU64 = AtomicU64::new(0);
    pub static COMMITTED: AtomicU64 = AtomicU64::new(0);
    pub static COMMITTED_AHEAD: AtomicU64 = AtomicU64::new(0);
    pub static CPUID: AtomicU64 = AtomicU64::new(0);
    pub static AHEAD_MPROTECT: AtomicU64 = AtomicU64::new(0);
    pub static AHEAD_MADVISE: AtomicU64 = AtomicU64::new(0);
    pub static ALL: [&AtomicU64; 8] = [
        &DIRECT_SYSCALLS,
        &TRAP_SYSCALLS,
        &PAGE_FAULTS,
        &COMMITTED,
        &COMMITTED_AHEAD,
        &CPUID,
        &AHEAD_MPROTECT,
        &AHEAD_MADVISE,
    ];
}

/// Copy the counters to the phase page at word `at`.
fn store_counters(at: usize) {
    let page = PHASE_PAGE.load(Ordering::Acquire) as usize;
    if page == 0 {
        return;
    }
    for (i, c) in count::ALL.iter().enumerate() {
        // Safety: the page was checked to lie outside the enclave when it
        // was mapped and stays mapped.
        let word = unsafe { &*((page + (at + i) * 8) as *const AtomicU64) };
        word.store(c.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

/// Mappings up to this size are committed when `mprotect()` makes them
/// accessible. Of a larger one only the top [`COMMIT_TOP`] bytes are: a
/// thread stack (lind's are 64 MiB, of which a thread uses little, since
/// wasmtime limits the wasm stack) is used from the top down, and a wasm
/// linear memory grown to its maximum at once is committed as the payload
/// says it uses parts of it (`madvise(MADV_WILLNEED)`), or on first touch.
const COMMIT_ON_MPROTECT_MAX: usize = 64 << 20;
const COMMIT_TOP: usize = 8 << 20;

/// An accessible `mmap()` up to this size (a buffer, code, a heap arena) is
/// committed at once. A larger one is a wasm linear memory, which lind maps
/// whole (4 GiB) and uses a little of: only its top [`COMMIT_TOP`] bytes are
/// committed (lind keeps a futex page there), the rest as lind says it uses
/// it.
const COMMIT_ON_MMAP_MAX: usize = 1 << 30;

const MADV_WILLNEED: c_int = 3;

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

/// A line of text formatted in the enclave, cut at 128 bytes.
struct Line {
    buf: [u8; 128],
    len: usize,
}

impl Line {
    const fn new() -> Self {
        Self {
            buf: [0; 128],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

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
    /// Handling a syscall taken through the direct entry, at CSSA 0.
    direct: bool,
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
    /// The host broke a rule, or the shim's state is not what it relies on:
    /// end the run as a refusal does. (sallyport's `attacked()` loops on
    /// `exit_group()`, which never ends a thread that came in through the
    /// direct entry: there it only arranges the return to the thread loop,
    /// so the keep would spin forever.)
    #[track_caller]
    fn attacked(&mut self) -> ! {
        let at = core::panic::Location::caller();
        self.fail_stop(format_args!("attacked ({}:{})", at.file(), at.line()))
    }

    fn sally(&mut self) -> sallyport::Result<()> {
        // Every request reaches the host through here. Once the keep is
        // sealed, only those the thread has a permit for do: whatever path
        // led here, a bug above the shim cannot reach the host.
        if SEALED.load(Ordering::Acquire) && !self.tcb.host_permit {
            let (kind, number) = (self.block[1], self.block[2]);
            self.sealed_refuse("host request of kind", kind as u64, number as u64);
        }

        // prevent earlier writes from being moved beyond this point
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);

        if self.direct {
            // No exception got us here, so there is no handler for the host
            // to finish: raise one. The handler entered for this `syscall`
            // asks the host to run the request in the block, which CSSA 0
            // and 1 share.
            unsafe {
                asm!("syscall", inlateout("rax") SYS_SALLY as u64 => _);
            }
        } else {
            unsafe {
                // Safety: Enclave exit and re-enter should have left all registers intact.
                asm!("syscall");
            }
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

        // Starting a thread on the host (the pool, or one more when no
        // thread waits in the enclave) is allowed in a sealed keep.
        if !POOL_STARTED.swap(true, Ordering::SeqCst) {
            self.permit(|h| h.start_thread_pool());
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

        // If no thread can be had, the run ends rather than the clone()
        // fails: a build step that saw EAGAIN could take another path (make
        // running fewer jobs, a program falling back to one thread), one the
        // host would have chosen by refusing the thread.
        let mut threads_free_guard = THREADS_FREE.write();

        let addr = if *threads_free_guard == 0 {
            debugln!(self, "[{tid}] allocating new thread");
            match self.permit(|h| h.thread_mem_alloc()) {
                Ok(tcs) => tcs as usize,
                Err(e) => {
                    drop(threads_free_guard);
                    self.fail_stop(format_args!("no memory for a new thread (error {e})"))
                }
            }
        } else {
            *threads_free_guard -= 1;
            0
        };

        drop(threads_free_guard);

        ptid.store(new_tid as _, Ordering::Relaxed);

        NEW_THREAD_QUEUE.write().push(new_thread).unwrap();

        let ret = self.permit(|h| h.spawn(addr));
        debugln!(self, "[{tid}] spawn() = {ret:#?}");
        if let Err(e) = ret {
            self.fail_stop(format_args!("the host did not start a thread (error {e})"));
        }

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
            self.return_to_main(status);

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
        addr: NonNull<c_void>,
        length: c_size_t,
        advice: c_int,
    ) -> sallyport::Result<()> {
        // The payload will use these pages: commit them now, from the
        // enclave, instead of on first touch through an exception.
        if advice == MADV_WILLNEED {
            let addr = addr.as_ptr() as usize;
            if addr & 0xfff != 0 {
                return Err(EINVAL);
            }
            let pages = (length + Page::SIZE - 1) / Page::SIZE;
            if pages == 0 {
                return Ok(());
            }
            let heap = HEAP.write();
            let start = Address::new(addr);
            let length = Offset::from_items(pages);
            if heap.contains(start, length).is_none() {
                return Err(ENOMEM);
            }
            let n = self.commit_ahead(&heap, start, length);
            count::AHEAD_MADVISE.fetch_add(n as u64, Ordering::Relaxed);
        }
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
        // An accessible mapping is committed now; a reservation (PROT_NONE)
        // only when parts of it are made accessible or used.
        let commit = prot & (PROT_READ | PROT_WRITE | PROT_EXEC) != 0;
        // TODO: https://github.com/enarx/enarx/issues/1892
        let prot = prot | PROT_READ;

        // MAP_NORESERVE (asked for by reservations, such as wasmtime's for
        // linear memories) means nothing for enclave memory.
        const MAP_NORESERVE: c_int = 0x4000;
        let flags = flags & !MAP_NORESERVE;
        if addr != 0 || len == 0 || fd != -1 || offset != 0 || flags != MAP_PRIVATE | MAP_ANONYMOUS
        {
            return Err(ENOTSUP);
        }

        if prot != 0 && !is_prot_allowed(prot) {
            return Err(EACCES);
        }

        let ret = self.do_mmap(prot, len, MMapStrategy::Lazy)?;
        if commit {
            let heap = HEAP.write();
            let pages = (len + Page::SIZE - 1) / Page::SIZE;
            let commit = if pages * Page::SIZE <= COMMIT_ON_MMAP_MAX {
                pages
            } else {
                COMMIT_TOP / Page::SIZE
            };
            let start = ret.as_ptr() as usize + (pages - commit) * Page::SIZE;
            self.commit_ahead(&heap, Address::new(start), Offset::from_items(commit));
        }
        Ok(ret)
    }

    fn mprotect(
        &mut self,
        _platform: &impl Platform,
        addr: NonNull<c_void>,
        len: c_size_t,
        prot: c_int,
    ) -> sallyport::Result<()> {
        let mut heap = HEAP.write();

        self.mprotect_unlocked(&mut heap, addr, len, prot)?;

        // Commit a mapping made accessible now (a thread stack below its
        // guard page, for instance) rather than on first touch.
        let pages = (len + Page::SIZE - 1) / Page::SIZE;
        if prot & (PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
            let commit = if pages * Page::SIZE <= COMMIT_ON_MPROTECT_MAX {
                pages
            } else {
                COMMIT_TOP / Page::SIZE
            };
            let start = addr.as_ptr() as usize + (pages - commit) * Page::SIZE;
            let n = self.commit_ahead(&heap, Address::new(start), Offset::from_items(commit));
            count::AHEAD_MPROTECT.fetch_add(n as u64, Ordering::Relaxed);
        }
        Ok(())
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
        // Console output is kept, in the enclave or in memory shared with
        // the host, until exit.
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
        // With the console in shared memory the host writes it out after
        // the keep exits; otherwise write it now.
        if !CONSOLE.lock().is_shared() {
            self.flush_console();
        }
        store_counters(PHASE_AT_EXIT);
        // Taken through the direct entry, at CSSA 0: return to the shim's
        // thread loop like exit(), which leaves the enclave for good, marked
        // for the host to exit the process, without an exception and a
        // request to the host.
        if self.direct && self.tcb.return_to_main.rip != 0 {
            self.tcb.exit_group = Some(status);
            self.return_to_main(status);
            // Not Ok: sallyport takes an exit_group() that returns Ok as an
            // attack. The registers set above are what the payload resumes
            // with, so the error never reaches it.
            return Err(EINTR);
        }
        self.permit(|h| h.execute(sys::ExitGroup { status }))??;
        self.attacked()
    }

    fn writev<T: ?Sized, U>(&mut self, fd: c_int, iovs: &T) -> sallyport::Result<c_size_t>
    where
        for<'b> &'b T: IntoIterator<Item = &'b U>,
        U: AsRef<[u8]>,
    {
        // Console output is kept like that of `write()`, in order with it.
        if fd == STDOUT_FILENO || fd == STDERR_FILENO {
            let mut written = 0;
            for iov in iovs {
                written += self.write(fd, iov.as_ref())?;
            }
            return Ok(written);
        }
        self.execute(sys::Writev { fd, iovs })?
            .unwrap_or_else(|| self.attacked())
    }

    fn clock_gettime(&mut self, clockid: clockid_t, tp: &mut timespec) -> sallyport::Result<()> {
        let realtime = match clockid {
            CLOCK_REALTIME | CLOCK_REALTIME_COARSE => true,
            CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME => {
                false
            }
            // The keep does not ask the host for its CPU time (glibc's
            // `clock()`) either: the monotonic clock, which starts at 0 with
            // the keep, stands in for it.
            CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID => false,
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
                        self.clock_ready();
                        Some(clock::realtime_to_monotonic(timespec_ns(t)))
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

/// Handle a syscall taken through the direct entry: run the #UD handler
/// on the registers the entry saved, as if they were the state save area
/// of a `syscall` instruction (see `crate::direct`).
///
/// # Safety
///
/// The GS base saved in `gpr` must be the calling thread's TCB, which
/// must have a block.
pub unsafe fn handle_direct(gpr: &mut GenPurposeRegs) {
    let tcb = &mut *(gpr.gsbase as *mut Tcb);
    let block =
        core::slice::from_raw_parts_mut(tcb.block as *mut usize, BLOCK_SIZE / size_of::<usize>());
    let mut ssa: StateSaveArea = core::mem::zeroed();
    ssa.gpr = *gpr;
    count::DIRECT_SYSCALLS.fetch_add(1, Ordering::Relaxed);
    {
        let mut h = Handler::new(&mut ssa, block, tcb, START.load(Ordering::Relaxed));
        h.direct = true;
        h.handle_syscall();
    }
    *gpr = ssa.gpr;
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
            direct: false,
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
                count::PAGE_FAULTS.fetch_add(1, Ordering::Relaxed);
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
                OP_SYSCALL => {
                    count::TRAP_SYSCALLS.fetch_add(1, Ordering::Relaxed);
                    h.handle_syscall()
                }
                OP_CPUID => {
                    count::CPUID.fetch_add(1, Ordering::Relaxed);
                    // The host answers `cpuid` for the enclave.
                    if SEALED.load(Ordering::Acquire) {
                        let (leaf, subleaf) = (h.ssa.gpr.rax, h.ssa.gpr.rcx);
                        h.sealed_refuse("cpuid", leaf, subleaf);
                    }
                    h.handle_cpuid()
                }
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
                count::PAGE_FAULTS.fetch_add(1, Ordering::Relaxed);
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

    /// Ask the host to map an open host file outside the enclave. The address
    /// it returns is refused unless the whole mapping lies outside the
    /// enclave: lind copies the dump archive into this memory, and a host
    /// that pointed it at the enclave would have the enclave overwrite itself.
    fn map_shm(&mut self, fd: c_int, length: usize, writable: bool) -> sallyport::Result<usize> {
        if length == 0 {
            return Err(EINVAL);
        }
        let addr = self.execute(MapShm {
            fd,
            length,
            writable,
        })??;
        let encl_start = shim_address();
        let encl_end = encl_start + ENCL_SIZE;
        match addr.checked_add(length) {
            Some(end) if addr != 0 && (end <= encl_start || addr >= encl_end) => {
                if writable {
                    DUMP_FD.store(fd, Ordering::Release);
                }
                Ok(addr)
            }
            _ => self.attacked(),
        }
    }

    /// Have the payload thread return to the shim's thread loop with
    /// `status` when the syscall is done.
    fn return_to_main(&mut self, status: c_int) {
        self.ssa.gpr.rsp = self.tcb.return_to_main.rsp;
        // The syscall handler will add 2 to rip to skip the syscall instruction (which we don't have anymore)
        self.ssa.gpr.rip = self.tcb.return_to_main.rip - 2;
        self.ssa.gpr.rbx = self.tcb.return_to_main.rbx;
        self.ssa.gpr.rbp = self.tcb.return_to_main.rbp;
        self.ssa.gpr.gsbase = self.tcb.return_to_main.gsbase;
        self.ssa.gpr.fsbase = self.tcb.return_to_main.fsbase;
        self.ssa.gpr.rcx = status as _;
    }

    /// Map anonymous host memory for `kind`, refusing an address that is
    /// not entirely outside the enclave.
    fn map_host_shared(&mut self, kind: usize, length: usize) -> sallyport::Result<usize> {
        let addr = self.execute(MapHostShared { kind, length })??;
        let encl_start = shim_address();
        let encl_end = encl_start + ENCL_SIZE;
        match addr.checked_add(length) {
            Some(end) if addr != 0 && (end <= encl_start || addr >= encl_end) => Ok(addr),
            _ => self.attacked(),
        }
    }

    /// Set up the memory shared with the host for the console and the phase
    /// page, once, and return the address of the direct syscall entry, or 0
    /// if this thread has no block (an older host) and so cannot use it.
    fn direct_init(&mut self) -> usize {
        if PHASE_PAGE.load(Ordering::Acquire) == 0 {
            if let Ok(addr) = self.map_host_shared(shared::PHASE, PHASE_SIZE) {
                PHASE_PAGE.store(addr as u64, Ordering::Release);
            }
            if let Ok(addr) = self.map_host_shared(shared::CONSOLE, CONSOLE_SIZE) {
                CONSOLE.lock().share(addr, CONSOLE_SIZE);
            }
        }
        if self.tcb.block == 0 {
            return 0;
        }
        crate::direct::enarx_direct_entry as usize
    }

    /// The payload entered phase `n`: copy the counters to the phase page
    /// and set the mark. If the host watches the page, wait (up to a second)
    /// for it to acknowledge the mark, so that it counts from this point.
    fn phase(&mut self, n: u64) {
        let page = PHASE_PAGE.load(Ordering::Acquire) as usize;
        if page == 0 {
            return;
        }
        let word = |i: usize| unsafe { &*((page + i * 8) as *const AtomicU64) };
        store_counters(PHASE_AT_PHASE);
        word(PHASE_MARK).store(n, Ordering::SeqCst);
        if word(PHASE_WATCH).load(Ordering::SeqCst) != 0 {
            let deadline = self.clock_ns(false) + NSEC_PER_SEC;
            let mut spins = 1;
            while word(PHASE_ACK).load(Ordering::SeqCst) != n && clock::now_ns(false) < deadline {
                spins = backoff(spins);
            }
        }
    }

    /// Commit the pages of `[addr, addr + length)` that are not committed
    /// yet, from the enclave: the kernel adds each page (EAUG) when
    /// EACCEPTCOPY first touches it, which the kernel resolves by itself
    /// without the host entering the enclave.
    fn commit_ahead(
        &mut self,
        heap: &Heap,
        addr: Address<usize, Page>,
        length: Offset<usize, Page>,
    ) -> usize {
        self.map_heap_on_host(heap);
        let n = self.mmap_guest(addr, length, RWX);
        count::COMMITTED_AHEAD.fetch_add(n as u64, Ordering::Relaxed);
        n
    }

    /// Write to a host file descriptor, bypassing the console buffer. A
    /// sealed keep writes to the console only.
    fn write_host(&mut self, fd: c_int, buf: &[u8]) -> sallyport::Result<c_size_t> {
        let ret = if fd == STDOUT_FILENO || fd == STDERR_FILENO {
            self.permit(|h| h.execute(sys::Write { fd, buf }))
        } else {
            self.execute(sys::Write { fd, buf })
        };
        ret?.unwrap_or_else(|| self.attacked())
    }

    /// Run `f` with this thread allowed to ask the host for things although
    /// the keep is sealed.
    fn permit<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let outer = core::mem::replace(&mut self.tcb.host_permit, true);
        let ret = f(self);
        self.tcb.host_permit = outer;
        ret
    }

    /// Seal the keep (see [`SEALED`]). Get first what serving the syscalls
    /// a sealed keep allows could still need the host for, so that it never
    /// does: the clock's anchor and the heap's mapping on the host.
    fn seal(&mut self) {
        if SEALED.load(Ordering::Acquire) {
            return;
        }
        self.clock_ready();
        self.map_heap_on_host(&HEAP.read());
        SEALED.store(true, Ordering::Release);
    }

    /// End the run: the sealed keep was asked for `what` (`a`, `b`), which
    /// it does not serve (see [`Self::fail_stop`]).
    fn sealed_refuse(&mut self, what: &str, a: u64, b: u64) -> ! {
        self.fail_stop(format_args!("sealed keep: refused {what} {a} ({b:#x})"))
    }

    /// End the whole run, because of `why`. Say so on the console, then ask
    /// the host to exit the process, the one thing a sealed keep still
    /// allows. Never returns, so nothing of the request that led here goes
    /// further, and the payload never sees an error.
    fn fail_stop(&mut self, why: core::fmt::Arguments<'_>) -> ! {
        self.tcb.host_permit = true;
        let mut line = Line::new();
        let _ = writeln!(line, "[shim] {why}, exiting");
        // Unless this thread holds the console lock: then it says nothing.
        if let Some(mut console) = CONSOLE.try_lock() {
            let kept = console.push(STDERR_FILENO as u8, line.as_bytes());
            let shared = console.is_shared();
            drop(console);
            // The host writes out the shared console at exit, not the other.
            if !shared {
                self.flush_console();
            }
            if !kept {
                let _ = self.write_host(STDERR_FILENO, line.as_bytes());
            }
        }
        store_counters(PHASE_AT_EXIT);
        loop {
            let _ = self.execute(sys::ExitGroup {
                status: FAIL_STOP_EXIT,
            });
        }
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
        self.clock_ready();
        clock::now_ns(realtime)
    }

    /// Anchor the clocks unless they are.
    fn clock_ready(&mut self) {
        if !clock::ready() {
            self.clock_init();
        }
    }

    /// Anchor the clocks at the TSC (see [`crate::clock`]). The host helps
    /// only to find out how fast the TSC counts; no clock reading carries a
    /// value it gives. This happens on the first clock read or when the
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
        // measured against the host's monotonic clock over ~20 ms. Either
        // way the host says it; a frequency out of range ends the run.
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
            while clock::rdtsc().wrapping_sub(c0) < 50_000_000 {
                core::hint::spin_loop();
            }
            let t1 = host(self, CLOCK_MONOTONIC);
            let c1 = clock::rdtsc();
            match t1.checked_sub(t0) {
                Some(ns) if ns > 0 => {
                    (c1.wrapping_sub(c0) as u128 * NSEC_PER_SEC as u128 / ns as u128) as u64
                }
                _ => self.attacked(),
            }
        };

        if !clock::init(hz) {
            debugln!(self, "clock: TSC {hz} Hz is out of range");
            self.attacked();
        }
        debugln!(self, "clock: TSC {hz} Hz");
    }

    /// Start the threads that wait in the enclave for `clone()`, and anchor
    /// the clock, while the keep is still starting up.
    fn start_thread_pool(&mut self) {
        self.clock_ready();
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

        // Refused before anything of the syscall is copied to the block,
        // which the host can read.
        if SEALED.load(Ordering::Acquire) && !sealed_allows(nr as i64, self.ssa.gpr.rdi) {
            let arg0 = self.ssa.gpr.rdi;
            self.sealed_refuse("syscall", nr as u64, arg0);
        }

        let usermemscope = UserMemScope;

        match nr as i64 {
            SYS_SALLY => {
                // A syscall that came through the direct entry needs the
                // host: its request is in the block already.
                let _ = self.sally();
                self.ssa.gpr.rax = 0;
            }
            SYS_DIRECT_INIT => {
                self.ssa.gpr.rax = self.direct_init() as u64;
                self.ssa.gpr.rdx = orig_rdx;
            }
            SYS_PHASE => {
                let n = self.ssa.gpr.rdi;
                if n >= 1 {
                    self.seal();
                }
                self.phase(n);
                self.ssa.gpr.rax = 0;
                self.ssa.gpr.rdx = orig_rdx;
            }
            SYS_CLOCK_EPOCH => {
                // The epoch the workload declares, once, before the build (a
                // sealed keep refused this above).
                let epoch_ns = self.ssa.gpr.rdi;
                self.clock_ready();
                let ret = if epoch_ns > i64::MAX as u64 {
                    -EINVAL
                } else if clock::declare_epoch(epoch_ns) {
                    0
                } else {
                    -EPERM
                };
                self.ssa.gpr.rax = ret as u64;
                self.ssa.gpr.rdx = orig_rdx;
            }
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
            SYS_MAPSHM => {
                let ret = self.map_shm(
                    self.ssa.gpr.rdi as c_int,
                    self.ssa.gpr.rsi as usize,
                    self.ssa.gpr.rdx != 0,
                );
                match ret {
                    Err(e) => self.ssa.gpr.rax = -e as u64,
                    Ok(addr) => {
                        self.ssa.gpr.rax = addr as u64;
                        self.ssa.gpr.rdx = orig_rdx;
                    }
                }
            }
            SYS_COMMITSHM => {
                let (fd, length) = (self.ssa.gpr.rdi as c_int, self.ssa.gpr.rsi as usize);
                // A sealed keep commits the dump, and only once.
                if SEALED.load(Ordering::Acquire)
                    && (fd != DUMP_FD.load(Ordering::Acquire)
                        || DUMP_COMMITTED.swap(true, Ordering::SeqCst))
                {
                    self.sealed_refuse("dump commit of fd", fd as u64, length as u64);
                }
                let ret = self
                    .permit(|h| h.execute(CommitShm { fd, length }))
                    .and_then(|r| r);
                match ret {
                    Err(e) => self.ssa.gpr.rax = -e as u64,
                    Ok(()) => {
                        self.ssa.gpr.rax = 0;
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
    ) -> usize {
        let zero_virt_addr = VirtAddr::new(ZERO.as_ptr() as u64);
        // # Safety
        //
        // The address must be page aligned.
        let zero_page_addr = unsafe { PageAddr::from_start_address_unchecked(zero_virt_addr) };

        let mut committed_now = 0;
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
            committed_now += 1;
        }
        count::COMMITTED.fetch_add(committed_now as u64, Ordering::Relaxed);
        committed_now
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

#[cfg(test)]
mod tests {
    use super::{sealed_allows, SYS_GETTID, SYS_PRCTL, SYS_PROCESS_VM_WRITEV, SYS_TKILL};
    use sallyport::item::enarxcall::{
        SYS_CLOCK_EPOCH, SYS_COMMITSHM, SYS_DIRECT_INIT, SYS_GETATT, SYS_GETKEY, SYS_MAPSHM,
        SYS_PHASE, SYS_SALLY,
    };
    use sallyport::libc::{
        SYS_brk, SYS_clock_gettime, SYS_clone, SYS_close, SYS_exit, SYS_exit_group, SYS_fcntl,
        SYS_fstat, SYS_futex, SYS_getegid, SYS_geteuid, SYS_getgid, SYS_getpid, SYS_getrandom,
        SYS_getuid, SYS_ioctl, SYS_lseek, SYS_madvise, SYS_mmap, SYS_mprotect, SYS_mremap,
        SYS_munmap, SYS_nanosleep, SYS_open, SYS_read, SYS_readv, SYS_rt_sigprocmask,
        SYS_sched_yield, SYS_set_tid_address, SYS_sigaltstack, SYS_write, SYS_writev,
    };

    #[test]
    fn sealed_keep_serves_the_whitelist() {
        for nr in [
            SYS_mmap,
            SYS_munmap,
            SYS_mprotect,
            SYS_mremap,
            SYS_brk,
            SYS_madvise,
            SYS_futex,
            SYS_clone,
            SYS_exit,
            SYS_exit_group,
            SYS_sched_yield,
            SYS_set_tid_address,
            SYS_rt_sigprocmask,
            SYS_sigaltstack,
            SYS_PRCTL,
            SYS_GETTID,
            SYS_TKILL,
            SYS_getpid,
            SYS_PROCESS_VM_WRITEV,
            SYS_getuid,
            SYS_geteuid,
            SYS_getgid,
            SYS_getegid,
            SYS_clock_gettime,
            SYS_getrandom,
            SYS_PHASE,
            SYS_COMMITSHM,
            SYS_SALLY,
        ] {
            assert!(sealed_allows(nr, 0), "syscall {nr}");
        }
    }

    #[test]
    fn sealed_keep_writes_the_console_only() {
        for nr in [SYS_write, SYS_writev] {
            assert!(sealed_allows(nr, 1));
            assert!(sealed_allows(nr, 2));
            for fd in [0, 3, (1 << 32) | 1, u64::MAX] {
                assert!(!sealed_allows(nr, fd), "syscall {nr} on fd {fd:#x}");
            }
        }
    }

    #[test]
    fn sealed_keep_answers_standard_stream_queries_only() {
        for nr in [SYS_fstat, SYS_fcntl, SYS_ioctl] {
            for fd in [0, 1, 2] {
                assert!(sealed_allows(nr, fd), "syscall {nr} on fd {fd}");
            }
            for fd in [3, (1 << 32) | 1, u64::MAX] {
                assert!(!sealed_allows(nr, fd), "syscall {nr} on fd {fd:#x}");
            }
        }
    }

    #[test]
    fn sealed_keep_keeps_its_epoch() {
        for arg0 in [0, 1, 1_760_659_200_000_000_000] {
            assert!(!sealed_allows(SYS_CLOCK_EPOCH, arg0));
        }
    }

    #[test]
    fn sealed_keep_takes_nothing_from_the_host() {
        for nr in [
            SYS_read,
            SYS_readv,
            SYS_lseek,
            SYS_open,
            SYS_close,
            SYS_nanosleep,
            SYS_MAPSHM,
            SYS_GETATT,
            SYS_GETKEY,
            SYS_DIRECT_INIT,
        ] {
            for arg0 in [0, 1, 2, 3] {
                assert!(!sealed_allows(nr, arg0), "syscall {nr} ({arg0})");
            }
        }
    }
}
