"""Tabulate a canary results directory (canary/time.sh, canary/vary.sh).

Every run directory <name>-<i>/ holds canary.txt (what the canary program
printed: field=value) and products.sha256. For each configuration (the name
without its run number) and each field: "same" if every run recorded the
same value, else the values. Products likewise, plus libcanary.a's ar
timestamps (symbol table / members), from dump/out/libcanary.a or else
artimes.txt.
Usage: report.py <results dir>   (markdown on stdout)
"""
import os, re, sys
from collections import OrderedDict

R = sys.argv[1]


def artimes(path):
    try:
        b = open(path, "rb").read()
    except OSError:
        return None
    pos, armap, members = 8, None, set()
    while pos + 60 <= len(b):
        h = b[pos:pos + 60]
        name, date, size = h[:16].decode().strip(), int(h[16:28]), int(h[48:58])
        if name in ("/", "/SYM64/"):
            armap = date
        elif name != "//":
            members.add(date)
        pos += 60 + size + (size & 1)
    return "%s / %s" % (armap, ",".join(str(t) for t in sorted(members)))


runs = OrderedDict()
for d in sorted(os.listdir(R)):
    p = os.path.join(R, d)
    m = re.match(r"(.+)-(\d+)$", d)
    if not (os.path.isdir(p) and m):
        continue
    fields = OrderedDict()
    try:
        for line in open(os.path.join(p, "canary.txt")):
            k, _, v = line.rstrip("\n").partition("=")
            fields[k] = v
    except OSError:
        fields["(no canary.txt)"] = "-"
    try:
        for line in open(os.path.join(p, "products.sha256")):
            h, f = line.split()
            fields["sha256 " + f] = h[:16]
    except OSError:
        pass
    a = artimes(os.path.join(p, "dump/out/libcanary.a"))
    if not a and os.path.exists(os.path.join(p, "artimes.txt")):
        # a system whose archive was not kept (AB: from its log)
        a = open(os.path.join(p, "artimes.txt")).read().strip()
    if a:
        fields["ar symbol table / members"] = a
    runs.setdefault(m.group(1), []).append((int(m.group(2)), fields))

for cfg, rs in runs.items():
    rs.sort()
    print("## %s (%d runs)\n" % (cfg, len(rs)))
    print("| field | %s |" % " | ".join("run %d" % i for i, _ in rs))
    print("|---|" + "---|" * len(rs))
    keys = list(OrderedDict.fromkeys(k for _, f in rs for k in f))
    for k in keys:
        vals = [f.get(k, "-") for _, f in rs]
        if len(set(vals)) == 1:
            print("| %s | same: `%s` |%s" % (k, vals[0], " |" * (len(rs) - 1)))
        else:
            print("| %s | %s |" % (k, " | ".join("`%s`" % v for v in vals)))
    print()
