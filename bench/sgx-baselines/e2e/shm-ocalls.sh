#!/bin/bash
# What the preloading-signing flow (lua-triseal-shm.sh) asks of the host,
# with a runtime built with triseal/stats-overlay.py. Four keeps, compared at
# the first console write (the console is kept in the keep until exit):
#   build  the Lua build: 345 MB bundle, two attestations, dump of lua/luac
#   probe  the same bundle and attestations, `make --version`, dump
#   small  a bundle of make alone, one attestation, `make --version`, dump
#   bare   a bundle of make alone, no attestation, no dump
# build - probe is the compile, probe - small what 344 MB more preload and a
# second attestation cost, small - bare the attestation and the dump.
# Usage: e2e/shm-ocalls.sh <enarx> <outdir>
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
E=${E:-$HOME/sgx-bench/e2e}
ENARX=$1
OUT=$2
mkdir -p "$OUT"

run() {
  local name=$1
  shift
  echo "== $name" >&2
  /usr/bin/time -f "TIME wall=%e user=%U sys=%S" env ENARX_SGX_STATS=1 ENARX="$ENARX" "$@" \
    "$HERE/lua-triseal-shm.sh" "$E/work-shm-count" >"$OUT/$name.out" 2>&1
  cp "$E/work-shm-count/build.log" "$OUT/$name.log"
}
run build
run probe PROBE=1
run small PROBE=1 SMALL=1
run bare PROBE=1 SMALL=1 NOATTEST=1 NODUMP=1

python3 - "$OUT" <<'PY'
import os, re, sys
out = sys.argv[1]
runs = ["build", "probe", "small", "bare"]

def snapshot(name):
    t = open(os.path.join(out, name + ".log"), errors="replace").read()
    m = re.search(r"-+ SGX stats at first console write for process \d+ -+\n(.*?)(?=\n-+ SGX stats|\Z)", t, re.S)
    if not m:
        m = re.search(r"-+ SGX stats for process \d+ -+\n(.*?)(?=\n-+ SGX stats|\Z)", t, re.S)
    b = m.group(1)
    res = {}
    for k, p in {"ocalls": r"ocalls \(sallyport batches\):\s+(\d+)", "  syscalls": r"^\s+syscalls:\s+(\d+)",
                 "  enarxcalls": r"enarxcalls:\s+(\d+)", "EENTER": r"EENTERs:\s+(\d+)",
                 "AEX #UD": r"#UD (\d+)", "AEX #PF": r"#PF (\d+)"}.items():
        if mm := re.search(p, b, re.M):
            res[k] = int(mm.group(1))
    if mm := re.search(r"enarxcall histogram \(name: count\):\n((?:\s+\w+: \d+\n?)+)", b):
        for name, n in re.findall(r"(\w+): (\d+)", mm.group(1)):
            res["    " + name] = int(n)
    if mm := re.search(r"syscall histogram \(nr: count\):\n((?:\s+\d+: \d+\n?)+)", b):
        for nr, n in re.findall(r"(\d+): (\d+)", mm.group(1)):
            res["    syscall " + nr] = int(n)
    return res

snaps = {r: snapshot(r) for r in runs}
keys = list(dict.fromkeys(k for r in runs for k in snaps[r]))
print("before the console flush")
print(f"  {'counter':24s}" + "".join(f"{r:>10s}" for r in runs) +
      f"{'build-probe':>13s}{'probe-small':>13s}{'small-bare':>12s}")
rows = []
for k in keys:
    v = [snaps[r].get(k, 0) for r in runs]
    d = [v[0] - v[1], v[1] - v[2], v[2] - v[3]]
    rows.append([k.strip()] + v + d)
    print(f"  {k:24s}" + "".join(f"{x:>10d}" for x in v) + f"{d[0]:>13d}{d[1]:>13d}{d[2]:>12d}")
with open(os.path.join(out, "ocalls.csv"), "w") as f:
    f.write("counter," + ",".join(runs) + ",build-probe,probe-small,small-bare\n")
    for r in rows:
        f.write(",".join(str(x) for x in r) + "\n")
for r in runs:
    t = open(os.path.join(out, r + ".out")).read()
    w = re.search(r"TIME wall=([\d.]+) user=([\d.]+) sys=([\d.]+)", t)
    print(f"{r}: wall {w.group(1)} s, user {w.group(2)} s, sys {w.group(3)} s" if w else f"{r}: ?")
PY
echo "results: $OUT"
