# Build a TriSeal tree with SGX transition statistics, for measuring only:
# copies stats.rs (the ENARX_SGX_STATS module from another enarx checkout)
# into the tree, adds an enarxcall histogram and a snapshot at the first
# console write to it, and hooks it into src/backend/sgx/thread.rs.
# Usage: triseal/stats-overlay.py <stats.rs> <TriSeal tree>
import sys
src_stats, root = sys.argv[1], sys.argv[2]
p = root + "/src/backend/sgx/stats.rs"
open(p, "w").write(open(src_stats).read())
s = open(p).read()
if "ENARXCALL_HIST" not in s:
    s = s.replace("static SYSCALL_OTHER: AtomicU64 = ZERO;",
"""static SYSCALL_OTHER: AtomicU64 = ZERO;
static ENARXCALL_HIST: [AtomicU64; 32] = [ZERO; 32];""")
    s = s.replace("/// Print the summary to stderr.",
"""/// Record one enarxcall executed by the host, by number.
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
        _ => "other",
    }
}

/// Print the summary to stderr.""")
    s = s.replace("""    let mut hist: Vec<(usize, u64)> = SYSCALL_HIST""",
"""    let mut ehist: Vec<(usize, u64)> = ENARXCALL_HIST
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

    let mut hist: Vec<(usize, u64)> = SYSCALL_HIST""")
    open(p, "w").write(s)

s = open(root + "/src/backend/sgx/stats.rs").read()
if "mark_console" not in s:
    s = s.replace("static ENARXCALL_HIST: [AtomicU64; 32] = [ZERO; 32];",
"""static ENARXCALL_HIST: [AtomicU64; 32] = [ZERO; 32];
static MARKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);""")
    s = s.replace("/// Print the summary to stderr.\npub fn dump() {",
"""/// Print the summary once, at the first write to the console (or at
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

fn dump_titled(title: &str) {""")
    s = s.replace('eprintln!("----- SGX stats for process {} -----", std::process::id());',
                  'eprintln!("----- SGX stats {title}for process {} -----", std::process::id());')
open(root + "/src/backend/sgx/stats.rs", "w").write(s)

p = root + "/src/backend/sgx/mod.rs"
m = open(p).read()
if "pub mod stats;" not in m:
    lines = m.split("\n")
    # after the last `mod ...;` declaration near the top
    idx = max(i for i, l in enumerate(lines[:40]) if l.startswith("mod ") or l.startswith("pub mod "))
    lines.insert(idx + 1, "pub mod stats;")
    open(p, "w").write("\n".join(lines))

p = root + "/src/backend/sgx/thread.rs"
t = open(p).read()
def rep(a, b):
    global t
    assert a in t, a
    t = t.replace(a, b, 1)
rep("use super::enarxcall::sgx_enarxcall;", "use super::enarxcall::sgx_enarxcall;\nuse super::stats;")
rep("use std::sync::Arc;", "use std::sync::atomic::Ordering::Relaxed;\nuse std::sync::Arc;")
rep("""        let mut run: Run = unsafe { MaybeUninit::zeroed().assume_init() };
        run.tcs = self.tcs as u64;
        let how = self.how;""", """        stats::init();

        let mut run: Run = unsafe { MaybeUninit::zeroed().assume_init() };
        run.tcs = self.tcs as u64;
        let how = self.how;
        stats::entry(how);""")
rep("""        let exit_status = exit_status as i32;
""", """        let exit_status = exit_status as i32;
        stats::exit(run.function as usize, run.vector);
""")
rep("""            if let (EENTER, ERESUME) = (how, self.how) {
                let block: Block""", """            if let (EENTER, ERESUME) = (how, self.how) {
                stats::SALLYPORT_BATCHES.fetch_add(1, Relaxed);
                let block: Block""")
rep("""                        Item::Gdbcall(_gdbcall, _data) => {
""", """                        Item::Gdbcall(_gdbcall, _data) => {
                            stats::GDBCALLS.fetch_add(1, Relaxed);
""")
rep("""                        Item::Enarxcall(enarxcall, data) => {
""", """                        Item::Enarxcall(enarxcall, data) => {
                            stats::enarxcall(enarxcall.num as usize);
""")
rep("""                            trace!("exit_group({code})");""", """                            stats::mark_console();
                            stats::syscall(*num);
                            trace!("exit_group({code})");""")
rep("""                        Item::Syscall(ref _syscall, ..) => {
""", """                        Item::Syscall(ref _syscall, ..) => {
                            if _syscall.num == libc::SYS_write as usize
                                && (_syscall.argv[0] == 1 || _syscall.argv[0] == 2)
                            {
                                stats::mark_console();
                            }
                            stats::syscall(_syscall.num);
""")
open(p, "w").write(t)
print("overlay applied")
