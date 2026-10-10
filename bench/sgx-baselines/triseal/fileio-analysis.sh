#!/bin/bash
# Break down where TriSeal's file I/O time goes. Every run is a fresh keep
# with ENARX_SGX_STATS=1, so enclave transitions can be attributed per op.
#
#   1. Counter diff: each file test at two iteration counts; the difference
#      divided by the extra operations gives EEXITs/AEXs/ocalls per op.
#      Done for the deployed (debug) imfs grate, a release build of it, and
#      no grate at all (rawposix straight to host files).
#   2. File size sweep: MB_BLOCKS = 1..1024 to expose the chunk-list walk.
#
# Build first: STATIC=1 triseal/build.sh microbench, and the release grate:
#   (cd <imfs-grate> && cargo lind_compile --release) and copy the cwasm to
#   triseal/grates/imfs-grate-release.cwasm
# Usage: triseal/fileio-analysis.sh      -> results/fileio-<timestamp>/
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
LIND=${LIND:-$HOME/lind-wasm}
ENARX=${ENARX:-$HOME/enarx/target/release/enarx}
BUILD=$HERE/triseal/build
declare -A GRATE=(
  [debug]=$LIND/lindfs/grates/imfs-grate.cwasm
  [release]=$HERE/triseal/grates/imfs-grate-release.cwasm
  [none]=$BUILD/microbench.cwasm
)
TESTS="file_lseek file_write_64 file_read_64 file_write_4k file_read_4k"
LOW=10000
HIGH=100000
OUT=$HERE/results/fileio-$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"

# keep <log name> <debug|release|none> <syscall iters> <MB_BLOCKS> <test...>
keep() {
  local name=$1 grate=$2 iters=$3 blocks=$4 args toml=$OUT/$1.toml
  shift 4
  args=$(printf ', "%s"' "$iters" 1 "$@")
  if [ "$grate" = none ]; then
    printf 'args = [%s]\n\n[env]\nMB_BLOCKS = "%s"\n' "${args#, }" "$blocks" >"$toml"
  else
    printf 'args = ["/microbench"%s]\n\n[env]\nMB_BLOCKS = "%s"\nPRELOADS = "/microbench=%s/microbench.cwasm"\n' \
      "$args" "$blocks" "$BUILD" >"$toml"
  fi
  echo "== $name" >&2
  ENARX_SGX_STATS=1 ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$toml" \
    timeout 900 "$ENARX" run "${GRATE[$grate]}" >"$OUT/$name.log" 2>&1
}

for g in debug release none; do
  for t in $TESTS; do
    for n in $LOW $HIGH; do
      keep "diff.$g.$t.$n" "$g" "$n" 256 "$t"
    done
  done
done
for g in debug release; do
  for b in 1 16 256 1024; do
    keep "sweep.$g.$b" "$g" "$HIGH" "$b" file_write_64 file_read_64 file_write_4k file_read_4k
  done
done

python3 - "$OUT" "$LOW" "$HIGH" <<'PY'
import glob, os, re, sys
out, low, high = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
PATS = {
    "eexit": r"# of EEXITs:\s+(\d+)",
    "aex_ud": r"#UD (\d+)",
    "aex_pf": r"#PF (\d+)",
    "ocalls": r"# of ocalls \(sallyport batches\):\s+(\d+)",
    "syscalls": r"^\s+syscalls:\s+(\d+)",
    "enarxcalls": r"enarxcalls:\s+(\d+)",
}

def parse(path):
    text = open(path, errors="replace").read()
    res = {k: int(m.group(1)) if (m := re.search(p, text, re.M)) else None
           for k, p in PATS.items()}
    res["mb"] = {m.group(1): m.group(2) for m in re.finditer(r"^@mb (\S+) \S+ \S+ (\S+)", text, re.M)}
    res["skip"] = re.findall(r"^@mb (\S+) skip (.*)", text, re.M)
    hist = re.search(r"syscall histogram.*?\n((?:\s+\d+: \d+\n)+)", text)
    res["hist"] = dict(map(int, l.split(":")) for l in hist.group(1).split("\n") if l.strip()) if hist else {}
    return res

# Each test runs n = iters/10 timed ops plus n/10 warm-up ops.
ops = 1.1 * (high - low) / 10
print(f"Per-operation cost ({high // 10} timed ops; counters = diff between {low} and {high} iters / {ops:.0f} ops)\n")
hdr = ["grate", "test", "ns/op", "EEXIT/op", "#UD/op", "#PF/op", "ocall/op", "syscall/op", "enarxcall/op", "top host syscalls/op"]
print("| " + " | ".join(hdr) + " |\n|" + "---|" * len(hdr))
for g in ("debug", "release", "none"):
    for t in "file_lseek file_write_64 file_read_64 file_write_4k file_read_4k".split():
        a, b = (os.path.join(out, f"diff.{g}.{t}.{n}.log") for n in (low, high))
        if not (os.path.exists(a) and os.path.exists(b)):
            continue
        pa, pb = parse(a), parse(b)
        ns = pb["mb"].get(t) or "; ".join(r for n, r in pb["skip"] if n == t) or "fail"
        cells = []
        for k in ("eexit", "aex_ud", "aex_pf", "ocalls", "syscalls", "enarxcalls"):
            cells.append("?" if pa[k] is None or pb[k] is None else f"{(pb[k] - pa[k]) / ops:.2f}")
        hd = {nr: (pb["hist"].get(nr, 0) - pa["hist"].get(nr, 0)) / ops for nr in set(pa["hist"]) | set(pb["hist"])}
        top = ", ".join(f"{nr}:{v:.2f}" for nr, v in sorted(hd.items(), key=lambda x: -x[1])[:3] if v >= 0.01)
        print(f"| {g} | {t} | {ns} | " + " | ".join(cells) + f" | {top} |")

print("\nFile size sweep (ns/op, MB_BLOCKS blocks before wrapping)\n")
tests = ["file_write_64", "file_read_64", "file_write_4k", "file_read_4k"]
print("| grate | blocks | " + " | ".join(tests) + " |\n|" + "---|" * (len(tests) + 2))
for g in ("debug", "release"):
    for blk in (1, 16, 256, 1024):
        p = os.path.join(out, f"sweep.{g}.{blk}.log")
        if os.path.exists(p):
            mb = parse(p)["mb"]
            print(f"| {g} | {blk} | " + " | ".join(mb.get(t, "fail") for t in tests) + " |")
PY
echo "results: $OUT"
