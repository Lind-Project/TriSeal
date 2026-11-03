"""Tabulate canary/vary.sh: for every variation, what changed against the
control build of the same system and workload (fileordering against
fileordering-control, which differs from it only in the order the tree was
created in; home against home-control, which passes the drivers their paths
as home does).
  canary  the fields of ecanary's output that differ (control -> varied),
          and whether ecanary itself differs
  lua     which of lua, luac, liblua.a differ
control-2 against control-1 shows what differs with no variation at all.
A system without home-control (AB) compares home with control-1; a variation
it could not apply has unavailable.txt instead of results.
Usage: vary-report.py <results dir>   (markdown on stdout)
"""
import os, sys

R = sys.argv[1]


def load(sys_, wl, name):
    d = os.path.join(R, "%s-%s-%s" % (sys_, wl, name))
    if not os.path.isdir(d):
        return None
    f, p = {}, {}
    try:
        for line in open(os.path.join(d, "canary.txt")):
            k, _, v = line.rstrip("\n").partition("=")
            f[k] = v
    except OSError:
        pass
    try:
        for line in open(os.path.join(d, "products.sha256")):
            h, n = line.split()
            p[n] = h
    except OSError:
        pass
    return f, p


def diff(sys_, wl, name):
    un = os.path.join(R, "%s-%s-%s" % (sys_, wl, name), "unavailable.txt")
    if os.path.exists(un):
        return open(un).read().strip()
    base = {"fileordering": "fileordering-control", "home": "home-control"}.get(name, "control-1")
    if load(sys_, wl, base) is None and name == "home":
        base = "control-1"
    a, b = load(sys_, wl, base), load(sys_, wl, name)
    if a is None or b is None:
        return None
    (fa, pa), (fb, pb) = a, b
    if not pb:
        return "**build failed**"
    if wl == "canary":
        ch = ["%s: `%s` → `%s`" % (k, fa.get(k, "-"), fb.get(k, "-"))
              for k in fa if fa.get(k) != fb.get(k)]
        prod = "ecanary differs" if pa.get("ecanary") != pb.get("ecanary") else "ecanary identical"
        return prod + ("; " + "; ".join(ch) if ch else "")
    ch = [n for n in ("lua", "luac", "liblua.a") if pa.get(n) != pb.get(n)]
    return ("differ: " + ", ".join(ch)) if ch else "identical"


names = []
for d in sorted(os.listdir(R)):
    if os.path.isdir(os.path.join(R, d)) and d.count("-") >= 2:
        n = d.split("-", 2)[2]
        if n not in names and n not in ("control-1", "fileordering-control", "home-control"):
            names.append(n)
order = ["control-2", "environment", "build_path", "user_group", "fileordering", "home", "kernel",
         "aslr", "num_cpus", "time", "tsc-rate", "umask", "locales", "timezone", "exec_path",
         "domain_host", "declared"]
names.sort(key=lambda n: order.index(n) if n in order else 99)
systems = [(s, h) for s, h in (("native", "native"), ("triseal", "TriSeal"), ("ab", "AB (ES)"))
           if any(d.startswith(s + "-") for d in os.listdir(R))]
for wl in ("canary", "lua"):
    print("## %s\n" % wl)
    print("| variation | %s |" % " | ".join(h for _, h in systems))
    print("|---|" + "---|" * len(systems))
    for n in names:
        cells = [diff(s, wl, n) for s, _ in systems]
        if all(c is None for c in cells):
            continue
        print("| %s | %s |" % (n, " | ".join(c if c is not None else "–" for c in cells)))
    print()
