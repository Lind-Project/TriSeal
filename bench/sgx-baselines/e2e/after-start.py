"""Summarize after-start.sh: what the Lua build asked of the host from its
start to its end, on TriSeal (from the phase mark to exit) and Gramine
(build minus probe). Writes after-start.csv and prints a table."""
import csv, os, re, sys

out = sys.argv[1]

TRISEAL = {
    "EENTER": r"# of EENTERs:\s+(\d+)",
    "ERESUME": r"# of ERESUMEs:\s+(\d+)",
    "EEXIT": r"# of EEXITs:\s+(\d+)",
    "AEX": r"# of AEXs:\s+(\d+)",
    "AEX #UD": r"#UD (\d+)",
    "AEX #PF": r"#PF (\d+)",
    "AEX other": r"other (\d+);",
    "ocalls (sallyport batches)": r"# of ocalls \(sallyport batches\):\s+(\d+)",
    "  of which syscalls": r"^\s+syscalls:\s+(\d+)",
    "  of which enarxcalls": r"^\s+enarxcalls:\s+(\d+)",
}


def triseal_block(text):
    res = {k: int(m.group(1)) for k, p in TRISEAL.items() if (m := re.search(p, text, re.M))}
    # shim counters: "name: at the mark / at exit"
    shim = {}
    for name, a, b in re.findall(r"^\s{4,6}(\S[^:\n]*): (\d+) / (\d+)$", text, re.M):
        shim[name.strip()] = (int(a), int(b))
    return res, shim


def triseal(path):
    t = open(path, errors="replace").read()
    mark = re.search(r"-+ SGX stats at phase 1 for process \d+ -+\n(.*?)(?=\n-+ SGX stats|\Z)", t, re.S)
    final = re.findall(r"-+ SGX stats for process \d+ -+\n(.*?)(?=\n-+ SGX stats|\Z)", t, re.S)
    if not mark or not final:
        return None
    m, _ = triseal_block(mark.group(1))
    e, shim = triseal_block(final[-1])
    return m, e, shim


def gramine(path):
    t = open(path, errors="replace").read()
    blocks = re.split(r"-+ Total SGX stats for process", t)[1:]
    res = {"enclaves (processes)": len(blocks)}
    for b in blocks:
        for k, p in {"EENTER": r"# of EENTERs:\s+(\d+)", "EEXIT": r"# of EEXITs:\s+(\d+)",
                     "AEX": r"# of AEXs:\s+(\d+)", "sync signals": r"# of sync signals:\s+(\d+)",
                     "async signals": r"# of async signals:\s+(\d+)"}.items():
            if m := re.search(p, b):
                res[k] = res.get(k, 0) + int(m.group(1))
    return res


rows = []
for setup in ("triseal-preloads", "triseal-shm"):
    path = os.path.join(out, setup + ".log")
    r = triseal(path) if os.path.exists(path) else None
    if not r:
        print(f"{setup}: no phase mark or no final summary in {path}")
        continue
    m, e, shim = r
    for k in TRISEAL:
        if k in e:
            rows.append({"system": setup, "counter": k, "start": m.get(k, ""), "end": e[k],
                         "during the build": e[k] - m.get(k, 0)})
    for k, (a, b) in shim.items():
        rows.append({"system": setup, "counter": "shim: " + k, "start": a, "end": b,
                     "during the build": b - a})

b, p = (gramine(f) if os.path.exists(f := os.path.join(out, f"gramine.{k}.log")) else {}
        for k in ("build", "probe"))
for k in dict.fromkeys(list(b) + list(p)):
    bv, pv = b.get(k), p.get(k)
    rows.append({"system": "gramine-encrypted", "counter": k,
                 "start": pv if pv is not None else "", "end": bv if bv is not None else "",
                 "during the build": bv - pv if bv is not None and pv is not None else ""})

with open(os.path.join(out, "after-start.csv"), "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=["system", "counter", "start", "end", "during the build"])
    w.writeheader()
    w.writerows(rows)

print(f"{'system':20} {'counter':58} {'start':>10} {'end':>10} {'build':>10}")
for r in rows:
    print(f"{r['system']:20} {r['counter']:58} {str(r['start']):>10} {str(r['end']):>10} "
          f"{str(r['during the build']):>10}")
print("(TriSeal: start = the phase mark when make starts, end = exit;"
      " Gramine: start = probe, end = build)")
