"""Host syscalls of each TEE (strace -f -c of the whole process tree) for a
single-file build, build minus probe: the build's requests to the host.
Usage: strace-summary.py <dir with strace.<system>.{build,probe}.txt>"""
import os, re, sys

d = sys.argv[1]


def load(path):
    res = {}
    for line in open(path):
        f = line.split()
        if len(f) >= 5 and re.match(r"^\d", f[0]):
            res[f[-1]] = int(f[3])
    return res


print(f"\n{'host syscalls (strace -f -c)':30} {'probe':>10} {'build':>10} {'the build':>10}  top syscalls of the build")
for system in ("triseal", "triseal-shm", "gramine-encrypted", "occlum"):
    b, p = (os.path.join(d, f"strace.{system}.{k}.txt") for k in ("build", "probe"))
    if not (os.path.exists(b) and os.path.exists(p)):
        continue
    b, p = load(b), load(p)
    diff = {k: b.get(k, 0) - p.get(k, 0) for k in set(b) | set(p) if k != "total"}
    top = ", ".join(f"{k} {v}" for k, v in sorted(diff.items(), key=lambda kv: -kv[1])[:6] if v > 0)
    print(f"{system:30} {p.get('total', 0):>10} {b.get('total', 0):>10} {b.get('total', 0) - p.get('total', 0):>10}  {top}")
