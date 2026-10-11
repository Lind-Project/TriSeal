"""Summarize e2e/sfp/coldwarm.sh: median (min-max) seconds of each kind of
run per system and program, and the cold build against native.
Usage: coldwarm-summary.py <coldwarm.csv>      (prints Markdown tables)"""
import csv, statistics, sys

KINDS = ("cold", "startup", "first", "warm")
SYSTEMS = ("native", "triseal", "triseal-shm", "gramine-encrypted", "gramine-allowed", "occlum")
runs = {}
bad = []
for r in csv.DictReader(open(sys.argv[1])):
    if r["kind"] == "twice-total" or not r["seconds"]:
        continue
    if r["kind"] != "startup" and r["ok"] == "0":
        bad.append(r)
        continue
    runs.setdefault((r["prog"], r["system"], r["kind"]), []).append(float(r["seconds"]))


def cell(v):
    if not v:
        return "-"
    m = statistics.median(v)
    return f"{m:.1f} ({min(v):.1f}-{max(v):.1f})" if len(v) > 1 else f"{m:.1f}"


for prog in dict.fromkeys(p for p, _, _ in runs):
    native = runs.get((prog, "native", "cold"))
    print(f"\n### {prog}\n")
    print("| system | cold | startup | first | warm | cold / native | n |")
    print("|---|---|---|---|---|---|---|")
    for s in SYSTEMS:
        if not any((prog, s, k) in runs for k in KINDS):
            continue
        cold = runs.get((prog, s, "cold"))
        ratio = f"{statistics.median(cold) / statistics.median(native):.2f}x" if cold and native else "-"
        n = len(cold) if cold else 0
        print(f"| {s} | " + " | ".join(cell(runs.get((prog, s, k))) for k in KINDS) + f" | {ratio} | {n} |")
if bad:
    print("\nfailed runs (left out):")
    for r in bad:
        print(f"  {r['system']} {r['prog']} run {r['run']} {r['kind']}")
