#!/bin/bash
# Count the ocalls of the Lua build after the compile starts, for a TriSeal
# runtime built with the SGX stats hooks (ENARX_SGX_STATS):
#   build  the full build, with NODUMP=1 (lua and luac stay in the keep)
#   probe  the same keep running `make --version` (PROBE=1, NODUMP=1)
# The zero-ocall runtime keeps console output in the enclave until exit,
# and the stats print a snapshot at the first console write, so build minus
# probe at that snapshot is everything the compile asked of the host. For
# runtimes that write to the console while running, compare the totals.
# Also records wall-clock and CPU time (user, sys) of each keep.
# Usage: e2e/zero-ocall.sh <enarx> <outdir>
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
ENARX=$1
OUT=$2
mkdir -p "$OUT"

for kind in build probe; do
  probe=0
  [ $kind = probe ] && probe=1
  echo "== $kind" >&2
  # GNU time writes to stderr: its -o file would take fd 3, which enarx
  # needs for its socket.
  /usr/bin/time -f "TIME wall=%e user=%U sys=%S" env ENARX_SGX_STATS=1 NODUMP=1 PROBE=$probe \
    ENARX="$ENARX" "$HERE/lua-triseal.sh" "$E/work-zo-count" >"$OUT/$kind.out" 2>&1
  cp "$E/work-zo-count/build.log" "$OUT/$kind.log"
done

python3 - "$OUT" <<'PY'
import os, re, sys
out = sys.argv[1]

def block(path, title):
    t = open(path, errors="replace").read()
    m = re.search(r"-+ SGX stats " + title + r"for process \d+ -+\n(.*?)(?=\n-+ SGX stats|\Z)", t, re.S)
    if not m:
        return None
    b = m.group(1)
    res = {}
    for k, p in {"EENTER": r"EENTERs:\s+(\d+)", "ERESUME": r"ERESUMEs:\s+(\d+)",
                 "EEXIT": r"EEXITs:\s+(\d+)", "AEX #UD": r"#UD (\d+)", "AEX #PF": r"#PF (\d+)",
                 "ocalls": r"ocalls \(sallyport batches\):\s+(\d+)",
                 "  syscalls": r"^\s+syscalls:\s+(\d+)", "  enarxcalls": r"enarxcalls:\s+(\d+)"}.items():
        if mm := re.search(p, b, re.M):
            res[k] = int(mm.group(1))
    if mm := re.search(r"enarxcall histogram \(name: count\):\n((?:\s+\w+: \d+\n?)+)", b):
        for name, n in re.findall(r"(\w+): (\d+)", mm.group(1)):
            res["    " + name] = int(n)
    if mm := re.search(r"syscall histogram \(nr: count\):\n((?:\s+\d+: \d+\n?)+)", b):
        for nr, n in re.findall(r"(\d+): (\d+)", mm.group(1)):
            res["    syscall " + nr] = int(n)
    return res

def times(path):
    m = re.search(r"TIME wall=([\d.]+) user=([\d.]+) sys=([\d.]+)", open(path).read())
    return m.groups() if m else ("?", "?", "?")

rows = []
for title, label in [("at first console write ", "before the console flush"), ("", "at exit")]:
    b = block(os.path.join(out, "build.log"), title)
    p = block(os.path.join(out, "probe.log"), title)
    if not b or not p:
        continue
    print(f"\n{label}")
    print(f"  {'counter':28s}{'build':>12s}{'probe':>12s}{'compile':>12s}")
    for k in dict.fromkeys(list(b) + list(p)):
        bv, pv = b.get(k, 0), p.get(k, 0)
        print(f"  {k:28s}{bv:>12d}{pv:>12d}{bv - pv:>12d}")
        rows.append((label, k, bv, pv, bv - pv))
with open(os.path.join(out, "counts.csv"), "w") as f:
    f.write("snapshot,counter,build,probe,compile\n")
    for r in rows:
        f.write(",".join(str(x).strip() for x in r) + "\n")
for kind in ("build", "probe"):
    w, u, s = times(os.path.join(out, kind + ".out"))
    print(f"{kind}: wall {w} s, user {u} s, sys {s} s")
PY
echo "results: $OUT"
