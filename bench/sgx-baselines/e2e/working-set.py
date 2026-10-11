"""Summarize e2e/working-set.sh: per build and runtime, the shim's page
counts at the IMFS phase mark (startup done) and at exit, in MB.
Usage: working-set.py <dir>"""
import os, re, sys

d = sys.argv[1]
KEYS = [
    ("pages committed", "committed (total)"),
    ("  peak of pages in use (lower bound)", "peak in use, lower bound"),
    ("  peak of pages in use (upper bound)", "peak in use, upper bound"),
    ("pages committed ahead of use", "of which committed ahead"),
    ("page faults handled by the shim (#PF)", "#PF handled by the shim (count)"),
]
mb = lambda pages: f"{pages * 4096 / 2**20:,.0f}"
print(f"{'build':8} {'runtime':12} {'':34} {'at mark':>10} {'at exit':>10}")
for p in ("lua", "oggenc", "gcc"):
    for rt in ("in-use", "working-set"):
        f = os.path.join(d, f"{rt}.{p}.log")
        if not os.path.exists(f):
            continue
        t = open(f, errors="replace").read()
        vals = {k: tuple(map(int, v.split(" / "))) for k, v in
                re.findall(r"^ {4}( ?.*?): (\d+ / \d+)$", t, re.M)}
        for key, label in KEYS:
            if key not in vals:
                continue
            a, b = vals[key]
            unit = "" if "count" in label else " MB"
            fa, fb = (str(a), str(b)) if "count" in label else (mb(a), mb(b))
            print(f"{p:8} {rt:12} {label + unit:34} {fa:>10} {fb:>10}")
        print()
