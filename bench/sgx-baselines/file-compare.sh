#!/bin/bash
# Compare file I/O on Gramine and TriSeal, both on SGX:
#   1. ns per call for each file test (src/microbench.c);
#   2. host syscalls per call, counted with `strace -f -c` for both systems
#      (the difference between two iteration counts over the extra calls), so
#      every ocall a call needs shows up the same way;
#   3. whether a file written by one process can be read by an exec'd child
#      (src/handoff.c), as a compiler pipeline needs.
#
# Build first:
#   (cd gramine && make APP=microbench && make APP=handoff)
#   STATIC=1 triseal/build.sh microbench && STATIC=1 triseal/build.sh handoff
# ENARX / IMFS_GRATE pick TriSeal's runtime and imfs grate; triseal-shm is
# TriSeal's full setup (triseal/full-run.sh, also INTOTO_GRATE): microbench
# in an attested bundle, the shim sealed, the evidence checked. Occlum runs in
# its container, so it gets times but no strace counts.
#   occlum/run-docker.sh ./build.sh microbench   (ramfs /tmp, SEFS root)
# Usage: ./file-compare.sh [config...]   -> results/filecmp-<timestamp>/
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
LIND=${LIND:-$HOME/lind-wasm}
ENARX=${ENARX:-$HOME/sgx-bench/e2e/ts/TriSeal36/target/release/enarx}
IMFS_GRATE=${IMFS_GRATE:-$HERE/triseal/grates/imfs-grate-clock.cwasm}
BUILD=$HERE/triseal/build
CONFIGS=${*:-native gramine-tmpfs gramine-allowed gramine-encrypted gramine-trusted triseal-imfs triseal-shm triseal-host occlum-ramfs occlum-sefs}
TESTS="file_open_close file_stat file_lseek file_write_64 file_read_64 file_write_4k file_read_4k"
TIMED=${TIMED:-100000}
LOW=${LOW:-10000}
HIGH=${HIGH:-100000}
OUT=${OUT:-$HERE/results/filecmp-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"

# triseal <imfs|none> <module> <env lines> <args...>: run a keep, argv from
# Enarx.toml. <env lines> are TOML lines for [env], each ending in a newline.
triseal() {
  local grate=$1 module=$2 envs=$3 toml args
  shift 3
  toml=$(mktemp -p "$OUT" XXXX.toml)
  args=$(printf ', "%s"' "$@")
  if [ "$grate" = none ]; then
    printf 'args = [%s]\n\n[env]\n%s' "${args#, }" "$envs" >"$toml"
    $TRACE env ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$toml" "$ENARX" run "$BUILD/$module.cwasm"
  else
    printf 'args = ["/%s"%s]\n\n[env]\n%sPRELOADS = "/%s=%s/%s.cwasm"\n' \
      "$module" "$args" "$envs" "$module" "$BUILD" "$module" >"$toml"
    $TRACE env ENARX_BACKEND=sgx ENARX_WASMCFGFILE="$toml" "$ENARX" run "$IMFS_GRATE"
  fi
}

# gramine <env...> -- <args...>
gramine() {
  local envs=()
  while [ "$1" != -- ]; do envs+=("$1"); shift; done
  shift
  (cd "$HERE/gramine" && $TRACE env "${envs[@]}" gramine-sgx "$@")
}

# micro <config> <syscall iters> <test...>: run microbench in one configuration.
micro() {
  local cfg=$1
  shift
  case $cfg in
    native)            (cd "$HERE/gramine/app" && $TRACE env MB_FILE=/dev/shm/mb.dat ./microbench "$1" 1 "${@:2}") ;;
    gramine-tmpfs)     gramine MB_FILE=/tmp/mb.dat -- microbench "$1" 1 "${@:2}" ;;
    gramine-allowed)   gramine MB_FILE=/allowed/mb.dat -- microbench "$1" 1 "${@:2}" ;;
    gramine-encrypted) gramine MB_FILE=/enc/mb.dat -- microbench "$1" 1 "${@:2}" ;;
    gramine-trusted)   gramine MB_FILE=/app/trusted.dat MB_NOFILL=1 -- microbench "$1" 1 "${@:2}" ;;
    triseal-imfs)      LIND_GRATE_WORKERS=1 triseal imfs microbench $'MB_FILE = "/tmp/mb.dat"\n' "$1" 1 "${@:2}" ;;
    triseal-shm)       TRACE="$TRACE" ENVS=$'MB_FILE = "/tmp/mb.dat"\n' IMFS_GRATE="$IMFS_GRATE" \
                         "$HERE/triseal/full-run.sh" "$OUT/full" "/microbench=$BUILD/microbench.cwasm" -- \
                         /microbench "$1" 1 "${@:2}" ;;
    triseal-host)      triseal none microbench $'MB_FILE = "/tmp/mb.dat"\n' "$1" 1 "${@:2}" ;;
    occlum-ramfs)      occlum /tmp/mb.dat "$@" ;;
    occlum-sefs)       occlum /root/mb.dat "$@" ;;
  esac
}

# occlum <file> <syscall iters> <test...>: ramfs is /tmp, SEFS is the root fs.
occlum() {
  local file=$1
  shift
  "$HERE/occlum/run-docker.sh" bash -c "cd inst && MB_FILE=$file occlum run /bin/microbench $1 1 ${*:2}"
}

# handoff <config>: parent writes a file, an exec'd child reads it back.
handoff() {
  case $1 in
    native)            (cd "$HERE/gramine/app" && ./handoff /dev/shm/handoff.dat) ;;
    gramine-tmpfs)     gramine -- handoff /tmp/handoff.dat ;;
    gramine-allowed)   gramine -- handoff /allowed/handoff.dat ;;
    gramine-encrypted) gramine -- handoff /enc/handoff.dat ;;
    triseal-imfs)      triseal imfs handoff '' /tmp/handoff.dat ;;
    triseal-shm)       IMFS_GRATE="$IMFS_GRATE" "$HERE/triseal/full-run.sh" "$OUT/full-h" \
                         "/handoff=$BUILD/handoff.cwasm" -- /handoff /tmp/handoff.dat ;;
    *)                 echo "@handoff n/a" ;;
  esac
}

TRACE=""
for cfg in $CONFIGS; do
  echo "== time $cfg" >&2
  # shellcheck disable=SC2086
  micro "$cfg" "$TIMED" $TESTS >"$OUT/time.$cfg.log" 2>&1
  echo "== handoff $cfg" >&2
  handoff "$cfg" >"$OUT/handoff.$cfg.log" 2>&1
done

for cfg in $CONFIGS; do
  case $cfg in occlum-*) continue ;; esac
  for t in $TESTS; do
    for n in $LOW $HIGH; do
      echo "== strace $cfg $t $n" >&2
      TRACE="strace -f -c -o $OUT/strace.$cfg.$t.$n.txt"
      micro "$cfg" "$n" "$t" >"$OUT/strace.$cfg.$t.$n.log" 2>&1
    done
  done
done

python3 - "$OUT" "$LOW" "$HIGH" "$CONFIGS" "$TESTS" <<'PY'
import csv, os, re, sys
out, low, high = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
configs, tests = sys.argv[4].split(), sys.argv[5].split()
ops = 1.1 * (high - low) / 10  # file tests run iters/10 timed + 10% warm-up

def mb(path):
    text = open(path, errors="replace").read() if os.path.exists(path) else ""
    res = {m.group(1): m.group(2) for m in re.finditer(r"^@mb (\S+) \S+ \S+ (\S+)", text, re.M)}
    res.update({m.group(1): "skip" for m in re.finditer(r"^@mb (\S+) skip", text, re.M)})
    return res

def strace(path):
    calls = {}
    if os.path.exists(path):
        for line in open(path):
            f = line.split()
            # % time, seconds, usecs/call, calls, [errors,] syscall
            if len(f) >= 5 and re.match(r"^\d", f[0]):
                calls[f[-1]] = int(f[3])
    return calls

def per_op(cfg, t):
    a = strace(f"{out}/strace.{cfg}.{t}.{low}.txt")
    b = strace(f"{out}/strace.{cfg}.{t}.{high}.txt")
    if "total" not in a or "total" not in b:
        return "?", ""
    total = (b["total"] - a["total"]) / ops
    diff = {k: (b.get(k, 0) - a.get(k, 0)) / ops for k in b if k != "total"}
    top = ", ".join(f"{k} {v:.1f}" for k, v in sorted(diff.items(), key=lambda x: -x[1]) if v >= 0.05)
    return f"{max(total, 0):.1f}", top

times = {cfg: mb(f"{out}/time.{cfg}.log") for cfg in configs}
print("ns per call (host syscalls per call, from strace)\n")
print("| test | " + " | ".join(configs) + " |\n|" + "---|" * (len(configs) + 1))
details = []
rows = []
for t in tests:
    cells = []
    for cfg in configs:
        sc, top = per_op(cfg, t)
        cells.append(f"{times[cfg].get(t, 'fail')} ({sc})")
        rows.append([cfg, t, times[cfg].get(t, "fail"), sc if sc != "?" else "", top])
        if top:
            details.append(f"{cfg} {t}: {top}")
    print(f"| {t} | " + " | ".join(cells) + " |")
with open(f"{out}/filecmp.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["config", "test", "ns_per_op", "host_syscalls_per_op", "host_syscalls_by_name"])
    w.writerows(rows)

print("\nHost syscalls per call by name:\n")
for d in details:
    print(f"- {d}")

print("\nFile handoff to an exec'd child:\n")
for cfg in configs:
    p = f"{out}/handoff.{cfg}.log"
    lines = [l.strip() for l in open(p, errors="replace") if l.startswith("@handoff")] if os.path.exists(p) else []
    print(f"- {cfg}: {'; '.join(lines) or 'no result'}")
PY
echo "results: $OUT"
