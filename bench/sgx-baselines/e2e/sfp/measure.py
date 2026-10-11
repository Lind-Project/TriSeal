"""Run a command in its own session and sample, every second, what its
processes cost: user and kernel CPU time, page faults (an enclave page the
kernel adds or loads back is a minor fault of the thread touching it), the
CPU time of ksgxd (the kernel's EPC reclaimer), how many threads are busy,
the clock of the CPU the busiest thread is on, and resident memory (peak RSS
of each process, VmHWM, and of all of them together; enclave memory is not
in RSS).
Usage: measure.py <out-prefix> <cmd> [args...]
Writes <out-prefix>.csv (one row a second) and prints a summary."""
import os, subprocess, sys, time

HZ = os.sysconf("SC_CLK_TCK")
out, cmd = sys.argv[1], sys.argv[2:]


def stat(path):
    try:
        s = open(path).read()
    except OSError:
        return None
    return s[s.rindex(")") + 2:].split()  # fields from 3 (state) on


def ksgxd():
    for pid in os.listdir("/proc"):
        if pid.isdigit():
            try:
                if open(f"/proc/{pid}/comm").read().strip() == "ksgxd":
                    return pid
            except OSError:
                pass


def kb(pid, field):
    try:
        for line in open(f"/proc/{pid}/status"):
            if line.startswith(field + ":"):
                return int(line.split()[1])
    except OSError:
        pass
    return 0


def sample(sid):
    procs = {"minflt": 0, "majflt": 0, "utime": 0, "stime": 0, "nproc": 0, "rss_kb": 0, "hwm_kb": 0}
    threads = {}
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        f = stat(f"/proc/{pid}/stat")
        if not f or int(f[3]) != sid:  # field 6: session
            continue
        procs["nproc"] += 1
        procs["minflt"] += int(f[7])
        procs["majflt"] += int(f[9])
        procs["rss_kb"] += kb(pid, "VmRSS")
        procs["hwm_kb"] = max(procs["hwm_kb"], kb(pid, "VmHWM"))
        try:
            tids = os.listdir(f"/proc/{pid}/task")
        except OSError:
            continue
        for tid in tids:
            t = stat(f"/proc/{pid}/task/{tid}/stat")
            if t:
                threads[tid] = (int(t[11]), int(t[12]), int(t[36]))  # utime, stime, cpu
    return procs, threads


def freq(cpu):
    try:
        return int(open(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq").read()) // 1000
    except OSError:
        return 0


kpid = ksgxd()
k0 = stat(f"/proc/{kpid}/stat") if kpid else None
start = time.time()
p = subprocess.Popen(cmd, start_new_session=True)
rows, last, dead = [], {}, {}
seen = {}
while True:
    try:
        rc = p.wait(timeout=1)
        done = True
    except subprocess.TimeoutExpired:
        done = False
    procs, threads = sample(p.pid)
    for tid, v in threads.items():
        seen[tid] = v
    busy, top, topcpu = 0, 0, -1
    for tid, (u, s, cpu) in threads.items():
        lu, ls, _ = last.get(tid, (u, s, cpu))
        d = (u - lu) + (s - ls)
        if d >= 0.5 * HZ:
            busy += 1
        if d > top:
            top, topcpu = d, cpu
    last = threads
    k = stat(f"/proc/{kpid}/stat") if kpid else None
    rows.append({"t": round(time.time() - start, 1), "nproc": procs["nproc"], "threads": len(threads),
                 "busy": busy, "minflt": procs["minflt"], "majflt": procs["majflt"],
                 "rss_mb": procs["rss_kb"] // 1024, "hwm_mb": procs["hwm_kb"] // 1024,
                 "utime": sum(u for u, _, _ in threads.values()) / HZ,
                 "stime": sum(s for _, s, _ in threads.values()) / HZ,
                 "ksgxd": (int(k[11]) + int(k[12]) - int(k0[11]) - int(k0[12])) / HZ if k else 0,
                 "topcpu_mhz": freq(topcpu) if topcpu >= 0 else 0})
    if done:
        break
wall = time.time() - start
with open(out + ".csv", "w") as f:
    f.write(",".join(rows[0]) + "\n")
    for r in rows:
        f.write(",".join(str(v) for v in r.values()) + "\n")
# totals over every thread ever seen (exited threads keep their last sample)
ut = sum(u for u, _, _ in seen.values()) / HZ
st = sum(s for _, s, _ in seen.values()) / HZ
flt = max(r["minflt"] for r in rows)
ks = rows[-1]["ksgxd"] if rows else 0
mhz = [r["topcpu_mhz"] for r in rows if r["topcpu_mhz"]]
busy = [r["busy"] for r in rows]
print(f"measure: wall {wall:.1f} s rc {rc}  user {ut:.1f} s  kernel {st:.1f} s  "
      f"minflt (max seen) {flt}  ksgxd {ks:.1f} s  busy threads mean {sum(busy)/len(busy):.1f}  "
      f"busiest thread's CPU {sum(mhz)/max(len(mhz),1):.0f} MHz")
print(f"measure: resident memory: peak of one process (VmHWM) {max(r['hwm_mb'] for r in rows)} MB, "
      f"peak of all together {max(r['rss_mb'] for r in rows)} MB")
top = sorted(seen.items(), key=lambda kv: -(kv[1][0] + kv[1][1]))[:6]
print("measure: busiest threads (user/kernel s): " +
      ", ".join(f"{tid} {u / HZ:.1f}/{s / HZ:.1f}" for tid, (u, s, _) in top))
sys.exit(rc)
