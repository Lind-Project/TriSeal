// SPDX-License-Identifier: Apache-2.0

//! Enclave transition statistics for the SGX backend.
//!
//! Every enclave entry and exit passes through [`super::thread::Thread::enter`],
//! which records it here. When the `ENARX_SGX_STATS` environment variable is
//! set, a summary formatted like Gramine's `sgx.enable_stats` output is
//! printed to stderr when the process exits (normally, or via SIGINT/SIGTERM).
//! If the variable holds a positive integer `N`, the summary is additionally
//! printed every `N` seconds while the keep is running.
//!
//! Note: AEXs caused by interrupts are resumed inside the vDSO and never reach
//! userspace, so the AEX count here only includes exception-driven AEXs
//! (`#UD` from `syscall`/`cpuid`, `#PF` from on-demand paging, ...).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Once;

use sgx::ssa::Vector;

const ZERO: AtomicU64 = AtomicU64::new(0);

/// Maximum syscall number tracked individually in the histogram.
const MAX_SYSCALL: usize = 512;

pub static EENTER: AtomicU64 = ZERO;
pub static ERESUME: AtomicU64 = ZERO;
pub static EEXIT: AtomicU64 = ZERO;
pub static AEX_UD: AtomicU64 = ZERO;
pub static AEX_PF: AtomicU64 = ZERO;
pub static AEX_OTHER: AtomicU64 = ZERO;

/// Number of times the host executed a sallyport block (≈ "ocalls").
pub static SALLYPORT_BATCHES: AtomicU64 = ZERO;
pub static SYSCALLS: AtomicU64 = ZERO;
pub static ENARXCALLS: AtomicU64 = ZERO;
pub static GDBCALLS: AtomicU64 = ZERO;
static SYSCALL_HIST: [AtomicU64; MAX_SYSCALL] = [ZERO; MAX_SYSCALL];
static SYSCALL_OTHER: AtomicU64 = ZERO;
static ENARXCALL_HIST: [AtomicU64; 32] = [ZERO; 32];
static MARKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static INIT: Once = Once::new();

/// Whether stats output is enabled (`ENARX_SGX_STATS` set).
pub fn enabled() -> bool {
    std::env::var_os("ENARX_SGX_STATS").is_some()
}

/// Periodic dump interval requested via `ENARX_SGX_STATS=<seconds>`, if any.
fn interval() -> Option<u64> {
    std::env::var("ENARX_SGX_STATS")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
}

extern "C" fn at_exit() {
    dump();
}

extern "C" fn on_signal(sig: libc::c_int) {
    // Not strictly async-signal-safe, but this is a debugging aid and the
    // process is about to die anyway.
    eprintln!("(SGX stats: caught signal {sig})");
    dump();
    unsafe { libc::_exit(128 + sig) }
}

/// Register the at-exit dump. Idempotent and cheap after the first call.
pub fn init() {
    INIT.call_once(|| {
        if !enabled() {
            return;
        }
        eprintln!(
            "(SGX stats enabled in process {}; summary printed at exit)",
            std::process::id()
        );
        unsafe {
            libc::atexit(at_exit);
            libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
            libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        }
        watch_phases();
        if let Some(secs) = interval() {
            std::thread::Builder::new()
                .name("sgx-stats".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(secs));
                    dump();
                })
                .expect("failed to spawn sgx-stats thread");
        }
    });
}

/// Watch the phase page the keep shares with the host (see `shared`): when
/// the workload marks a phase, print the summary and acknowledge the mark,
/// which the keep waits for, so that what follows the mark can be told from
/// what came before it.
fn watch_phases() {
    std::thread::Builder::new()
        .name("sgx-phase".into())
        .spawn(|| {
            use super::shared::phase::{ACK, PHASE, WATCH};
            use std::sync::atomic::Ordering::SeqCst;
            let page = loop {
                if let Some(page) = super::shared::phase_page() {
                    break page;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            page[WATCH].store(1, SeqCst);
            let mut last = 0;
            loop {
                let phase = page[PHASE].load(SeqCst);
                if phase != last {
                    dump_titled(&format!("at phase {phase} "));
                    page[ACK].store(phase, SeqCst);
                    last = phase;
                }
                std::thread::sleep(std::time::Duration::from_micros(50));
            }
        })
        .expect("failed to spawn sgx-phase thread");
}

/// Print what the shim counted in the enclave (see `shared::phase`).
fn print_shim_counters() {
    let Some(page) = super::shared::phase_page() else {
        return;
    };
    use super::shared::phase::{AT_EXIT, AT_PHASE, COUNTERS, NAMES, PHASE, WORKING_SET};
    eprintln!(
        "  shim counters (at the phase {} mark / at exit):",
        page[PHASE].load(Relaxed)
    );
    for i in 0..COUNTERS {
        let at_phase = page[AT_PHASE + i].load(Relaxed);
        let at_exit = page[AT_EXIT + i].load(Relaxed);
        if NAMES[i].is_empty() || (i >= WORKING_SET && at_phase == 0 && at_exit == 0) {
            continue;
        }
        eprintln!("    {}: {} / {}", NAMES[i], at_phase, at_exit);
    }
}

/// Record an enclave entry via `how` (`EENTER` or `ERESUME`).
#[inline]
pub fn entry(how: usize) {
    if how == sgx::enclu::EENTER {
        EENTER.fetch_add(1, Relaxed);
    } else {
        ERESUME.fetch_add(1, Relaxed);
    }
}

/// Record an enclave exit. `function` is `run.function`; if it is not `EEXIT`
/// the exit was an AEX with the given exception `vector`.
#[inline]
pub fn exit(function: usize, vector: Vector) {
    if function == sgx::enclu::EEXIT {
        EEXIT.fetch_add(1, Relaxed);
    } else {
        match vector {
            Vector::InvalidOpcode => &AEX_UD,
            Vector::Page => &AEX_PF,
            _ => &AEX_OTHER,
        }
        .fetch_add(1, Relaxed);
    }
}

/// Record one syscall proxied to the host.
#[inline]
pub fn syscall(num: usize) {
    SYSCALLS.fetch_add(1, Relaxed);
    match SYSCALL_HIST.get(num) {
        Some(c) => c.fetch_add(1, Relaxed),
        None => SYSCALL_OTHER.fetch_add(1, Relaxed),
    };
}

/// Record one enarxcall executed by the host, by number.
#[inline]
pub fn enarxcall(num: usize) {
    ENARXCALLS.fetch_add(1, Relaxed);
    if let Some(c) = ENARXCALL_HIST.get(num) {
        c.fetch_add(1, Relaxed);
    }
}

fn enarxcall_name(num: usize) -> &'static str {
    match num {
        0x00 => "MemInfo",
        0x01 => "BalloonMemory",
        0x02 => "Cpuid",
        0x03 => "GetSgxQuote",
        0x04 => "GetSgxTargetInfo",
        0x05 => "GetSgxQuoteSize",
        0x06 => "GetSnpVcek",
        0x07 => "MmapHost",
        0x08 => "MprotectHost",
        0x09 => "MunmapHost",
        0x10 => "SgxModifyPageType",
        0x11 => "Park",
        0x12 => "UnPark",
        0x13 => "Spawn",
        0x14 => "NewSallyport",
        0x15 => "MapShm",
        0x16 => "CommitShm",
        _ => "other",
    }
}

/// Print the summary once, at the first write to the console (or at
/// exit_group if there is none): with the console kept in the enclave until
/// exit, everything before it is the workload.
pub fn mark_console() {
    if enabled() && !MARKED.swap(true, Relaxed) {
        dump_titled("at first console write ");
    }
}

/// Print the summary to stderr.
pub fn dump() {
    dump_titled("");
}

fn dump_titled(title: &str) {
    let aex_ud = AEX_UD.load(Relaxed);
    let aex_pf = AEX_PF.load(Relaxed);
    let aex_other = AEX_OTHER.load(Relaxed);
    let aex = aex_ud + aex_pf + aex_other;
    let eenter = EENTER.load(Relaxed);
    let eresume = ERESUME.load(Relaxed);

    eprintln!(
        "----- SGX stats {title}for process {} -----",
        std::process::id()
    );
    eprintln!("  # of EENTERs:        {}", eenter);
    eprintln!("  # of ERESUMEs:       {}", eresume);
    eprintln!("  # of EEXITs:         {}", EEXIT.load(Relaxed));
    eprintln!("  # of AEXs:           {aex} (#UD {aex_ud}, #PF {aex_pf}, other {aex_other}; interrupt AEXs not visible)");
    eprintln!("  # of sync signals:   {aex}");
    eprintln!("  # of async signals:  0");
    eprintln!("  # of enclave entries: {}", eenter + eresume);
    eprintln!(
        "  # of ocalls (sallyport batches): {}",
        SALLYPORT_BATCHES.load(Relaxed)
    );
    eprintln!("    syscalls:          {}", SYSCALLS.load(Relaxed));
    eprintln!("    enarxcalls:        {}", ENARXCALLS.load(Relaxed));
    eprintln!("    gdbcalls:          {}", GDBCALLS.load(Relaxed));

    let mut ehist: Vec<(usize, u64)> = ENARXCALL_HIST
        .iter()
        .enumerate()
        .map(|(n, c)| (n, c.load(Relaxed)))
        .filter(|(_, c)| *c > 0)
        .collect();
    ehist.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if !ehist.is_empty() {
        eprintln!("  enarxcall histogram (name: count):");
        for (n, c) in ehist {
            eprintln!("    {}: {c}", enarxcall_name(n));
        }
    }

    let mut hist: Vec<(usize, u64)> = SYSCALL_HIST
        .iter()
        .enumerate()
        .map(|(n, c)| (n, c.load(Relaxed)))
        .filter(|(_, c)| *c > 0)
        .collect();
    hist.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if !hist.is_empty() || SYSCALL_OTHER.load(Relaxed) > 0 {
        eprintln!("  syscall histogram (nr: count):");
        for (n, c) in hist {
            eprintln!("    {n:>4}: {c}");
        }
        let other = SYSCALL_OTHER.load(Relaxed);
        if other > 0 {
            eprintln!("    >{}: {other}", MAX_SYSCALL - 1);
        }
    }
    print_shim_counters();
}
