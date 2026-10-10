"""Summarize counts.sh logs into counts.csv: build, probe and their
difference (the compile) for every counter."""
import csv, os, re, sys
from collections import defaultdict

out = sys.argv[1]

# x86-64 numbers that show up in the shim's syscall histogram.
SYSCALLS = {0: "read", 1: "write", 2: "open", 3: "close", 4: "stat", 5: "fstat",
            7: "poll", 8: "lseek", 9: "mmap", 11: "munmap", 16: "ioctl",
            17: "pread64", 18: "pwrite64", 20: "writev", 32: "dup", 33: "dup2",
            45: "recvfrom", 72: "fcntl", 202: "futex", 217: "getdents64",
            228: "clock_gettime", 231: "exit_group", 257: "openat",
            262: "newfstatat", 290: "eventfd2", 318: "getrandom"}

def triseal_stats(path):
    t = open(path, errors="replace").read()
    pats = {
        "EENTER": r"# of EENTERs:\s+(\d+)",
        "ERESUME": r"# of ERESUMEs:\s+(\d+)",
        "EEXIT": r"# of EEXITs:\s+(\d+)",
        "AEX": r"# of AEXs:\s+(\d+)",
        "AEX #UD": r"#UD (\d+)",
        "AEX #PF": r"#PF (\d+)",
        "ocall (sallyport batch)": r"# of ocalls \(sallyport batches\):\s+(\d+)",
        "  of which syscall": r"^\s+syscalls:\s+(\d+)",
        "  of which enarxcall": r"enarxcalls:\s+(\d+)",
        "EENTER+ERESUME": r"# of enclave entries:\s+(\d+)",
    }
    res = {k: int(m.group(1)) for k, p in pats.items() if (m := re.search(p, t, re.M))}
    if m := re.search(r"syscall histogram \(nr: count\):\n((?:\s+\d+: \d+\n?)+)", t):
        for nr, n in re.findall(r"(\d+): (\d+)", m.group(1)):
            res[f"  syscall {SYSCALLS.get(int(nr), 'nr ' + nr)}"] = int(n)
    return res

def gramine_stats(path):
    # Every enclave (one per process) prints a total when it exits.
    t = open(path, errors="replace").read()
    blocks = re.split(r"-+ Total SGX stats for process", t)[1:]
    res = defaultdict(int)
    res["processes (enclaves)"] = len(blocks)
    for b in blocks:
        for k, p in {"EENTER": r"# of EENTERs:\s+(\d+)", "EEXIT": r"# of EEXITs:\s+(\d+)",
                     "AEX": r"# of AEXs:\s+(\d+)", "sync signals": r"# of sync signals:\s+(\d+)",
                     "async signals": r"# of async signals:\s+(\d+)"}.items():
            if m := re.search(p, b):
                res[k] += int(m.group(1))
    return dict(res)

def strace(path):
    res = {}
    for line in open(path):
        f = line.split()
        if len(f) >= 5 and re.match(r"^\d", f[0]):
            res[f[-1]] = int(f[3])
    return {"host syscalls (total)": res.pop("total", 0), **{f"  {k}": v for k, v in res.items()}}

rows = []
for system, source, fn, stem in [
    ("triseal", "ENARX_SGX_STATS", triseal_stats, "triseal-stats.{}.log"),
    ("gramine-encrypted", "sgx.enable_stats (debug Gramine)", gramine_stats, "gramine-stats.{}.log"),
    ("triseal", "strace -f -c", strace, "triseal-strace.{}.txt"),
    ("gramine-encrypted", "strace -f -c", strace, "gramine-strace.{}.txt"),
]:
    b, p = (fn(f) if os.path.exists(f := os.path.join(out, stem.format(k))) else {}
            for k in ("build", "probe"))
    if fn is strace:
        # The total, then the syscalls the build makes most.
        top = sorted((k for k in b if k.startswith("  ")), key=lambda k: -b[k])[:10]
        keys = ["host syscalls (total)"] + top
        b = {k: b.get(k, 0) for k in keys}
        p = {k: p.get(k, 0) for k in keys} if p else {}
    else:
        keys = list(dict.fromkeys(list(b) + list(p)))
    for k in keys:
        bv, pv = b.get(k), p.get(k)
        diff = bv - pv if bv is not None and pv is not None else ""
        rows.append({"system": system, "source": source, "counter": k,
                     "build": bv if bv is not None else "", "probe": pv if pv is not None else "",
                     "compile": diff})

with open(os.path.join(out, "counts.csv"), "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=["system", "source", "counter", "build", "probe", "compile"])
    w.writeheader()
    w.writerows(rows)
cur = None
for r in rows:
    if (r["system"], r["source"]) != cur:
        cur = (r["system"], r["source"])
        print(f"\n{r['system']} / {r['source']}")
        print(f"  {'counter':28s}{'build':>14s}{'probe':>14s}{'compile':>14s}")
    print(f"  {r['counter']:28s}{str(r['build']):>14s}{str(r['probe']):>14s}{str(r['compile']):>14s}")
