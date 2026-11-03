#!/bin/bash
# Benchmark: reprotest-style variations. reprotest's variations, one at a
# time, against control builds, natively and in TriSeal:
#   native   the variation applied to the build itself (reprotest's way)
#   triseal  the variation applied to the host side of the keep (mkpreload,
#            enarx and everything they run); the invocation stays the same.
#            TriSeal runs its reproducible configuration: full setup with the
#            logical clock (epoch $EPOCH declared) above make
# Two workloads per run:
#   canary   canary/proj `make env`: ecanary records what the build sees
#            (probe env: environment, uname, user, host name, CPUs, umask,
#            working directory, readdir order, time zone, locale,
#            addresses) and __FILE__
#   lua      Lua 5.5.1 with its Makefile unchanged (ar rcu):
#            lua, luac, liblua.a (e2e/lua-native.sh, e2e/lua-triseal-shm.sh)
# Variations (reprotest's names):
#   environment   + CAPTURE_ENVIRONMENT=i_capture_the_environment
#   build_path    another work directory
#   user_group    another uid:gid: 0:0 instead of ours (docker --user; the
#                 bench tree is not readable by any other uid)
#   fileordering  the source tree and the build directory on tmpfs, created
#                 in reverse order (tmpfs lists entries in the order they were
#                 created); its control is the same tree created in order, at
#                 the same paths
#   home          HOME=/nonexistent/first-build; against home-control, which
#                 like it passes the drivers their paths (E, MKPRELOAD)
#                 instead of letting them derive them from HOME
#   kernel        setarch --uname-2.6
#   aslr          setarch -R (ASLR off)
#   num_cpus      taskset -c 0 (one CPU of 32)
#   time          libfaketime +7 days (Ubuntu's libfaketime, LD_PRELOAD;
#                 reprotest's +373 days makes Enarx refuse to start: its
#                 cached Intel TCB report is then past its nextUpdate);
#                 TriSeal also: tsc-rate, the keep's clock rate x3
#                 (ENARX_TEST_TSC_SCALE, canary/enarx-host, same MRENCLAVE)
#   umask         0077
#   locales       LANG=LC_ALL=en_GB.utf8
#   timezone      TZ=GMT+12
#   exec_path     PATH+=/i_capture_the_path
#   domain_host   another host name (docker --hostname)
#   + TriSeal declared: TZ, LANG, HOME and the junk variable declared in the
#     invocation instead: the build sees them, and run-start records them
# Usage: canary/vary.sh [results dir]   (SYSTEMS, default "native triseal";
#        VARIATIONS, default all)
set -u
K=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
B=$HOME/sgx-bench; E=$B/e2e; C=$E/clang
R=${1:-$B/results/canary-vary-$(date +%Y%m%d-%H%M%S)}; mkdir -p "$R"
EPOCH=${EPOCH:-1700000000}
FT=$B/tools/faketime/root/usr/lib/x86_64-linux-gnu/faketime/libfaketime.so.1
SYSTEMS=${SYSTEMS:-native triseal}
ALL="control-1 control-2 environment build_path user_group fileordering-control fileordering home-control home kernel aslr num_cpus time tsc-rate umask locales timezone exec_path domain_host declared"
VARIATIONS=${VARIATIONS:-$ALL}
{ echo "start $(date)"; echo "enarx $K/enarx-default $(sha256sum < $K/enarx-default | cut -c1-16)"
  echo "epoch $EPOCH"; echo "host: $(uname -r), $(nproc) CPUs, umask $(umask), LANG=${LANG:-}, TZ=${TZ:-}"
  echo "variations: $VARIATIONS"; } >> "$R/env.txt"

# The TriSeal configuration: the logical clock, as canary/time.sh's
# triseal-clock; Lua as e2e/clang/clock.sh with gcc.
CLOCKENV=(IMFS_GRATE=$C/ebc/imfs-grate-clock.cwasm CLOCK=$C/ebc/logical-clock-grate.cwasm
  ENARX=$K/enarx-default INTOTO_GRATE=$B/triseal/ebc/intoto-grate.cwasm ROOT=$E/root-ebc DUMP_LIB=1
  # uname and the host name are refused once the keep is sealed (syscall
  # 63): a build that reads them stops (canary-classes). So TriSeal's
  # canary reads every other class.
  "ENV_CLASSES=vars ids cpus umask cwd readdir tz locale addr")
DECL="LOGICAL_CLOCK_EPOCH = \"$EPOCH\""

# Docker for the variations that need another uid or host name: the host's
# Ubuntu, the bench tree and /dev/shm mounted at the same paths, the SGX
# devices, the AESM socket for TriSeal's quote and the Intel certificate
# cache Enarx checks for.
dock() {  # <docker args...> :: <command...>
  local a=()
  while [ "$1" != :: ]; do a+=("$1"); shift; done; shift
  docker run --rm "${a[@]}" -v "$B:$B" -v /dev/shm:/dev/shm -v /var/run/aesmd:/var/run/aesmd -v /var/cache/intel-sgx:/var/cache/intel-sgx:ro \
    --device /dev/sgx_enclave --device /dev/sgx_provision -e HOME="$HOME" -e LANG="${LANG:-}" \
    -w "$B" canary-runner "$@"
}

# One build: <system> <workload> <name> <workdir> <wrapper...> -- [env...]
build() {
  local sys=$1 wl=$2 name=$3 w=$4; shift 4
  local wrap=()
  while [ "$1" != -- ]; do wrap+=("$1"); shift; done; shift
  local drv out
  if [ "$wl" = canary ]; then
    drv=("$K/$sys.sh" "$w")
  elif [ "$sys" = native ]; then
    drv=("$E/lua-native.sh" "$w")
  else
    drv=("$E/lua-triseal-shm.sh" "$w")
  fi
  local envs=(TARGET=env "$@")
  [ "$sys" = triseal ] && envs=("${CLOCKENV[@]}" GUEST_ENV="$DECL${DECLX:+
$DECLX}" "${envs[@]}")
  out=$R/$sys-$wl-$name
  mkdir -p "$out"
  # Nothing of an earlier build may be taken for this one's if it fails.
  rm -rf "$w"
  "${wrap[@]}" env "${envs[@]}" bash "${drv[@]}" > "$out/build.out" 2>&1
  # Collect what to compare.
  if [ "$wl" = canary ]; then
    cp "$w/canary.txt" "$w/products.sha256" "$out/" 2>/dev/null
    [ -f "$w/build.toml" ] && cp "$w/build.toml" "$out/"
  else
    local d=$w/src
    [ "$sys" = triseal ] && d=$w/dump/out
    (cd "$d" && sha256sum lua luac liblua.a) > "$out/products.sha256" 2>/dev/null
    [ -f "$w/build.toml" ] && cp "$w/build.toml" "$out/"
  fi
  printf '%-8s %-6s %-22s %s | %s\n' "$sys" "$wl" "$name" "$(grep -aE '^wall' "$out/build.out" | head -1) $(grep -ac '^evidence verified' "$out/build.out") verified" \
    "$(cut -c1-12 "$out/products.sha256" 2>/dev/null | tr '\n' ' ')" | tee -a "$R/summary.txt"
}

# The fileordering trees: the same files at the same path on tmpfs, created
# in order or in reverse order.
mktree() {  # <src> <dst> <sort flags>
  rm -rf "$2"; mkdir -p "$2"
  (cd "$1" && find . -type d | sort) | while read -r d; do mkdir -p "$2/$d"; done
  (cd "$1" && find . -type f | sort $3) | while read -r f; do cp -p "$1/$f" "$2/$f"; done
}

for v in $VARIATIONS; do
  for sys in $SYSTEMS; do
    for wl in canary lua; do
      W=$E/work-vary-$sys-$wl
      wrap=(); envs=(); DECLX=""
      case $v in
        control-*|fileordering-control|fileordering) ;;
        home-control) envs=(E=$E MKPRELOAD=$B/grates-attest/tools/mkpreload/target/release/mkpreload) ;;
        environment) envs=(CAPTURE_ENVIRONMENT=i_capture_the_environment) ;;
        build_path) W=$E/work-vary-elsewhere/deeper/path/$sys-$wl ;;
        user_group) wrap=(dock --user 0:0 ::) ;;
        home) envs=(E=$E MKPRELOAD=$B/grates-attest/tools/mkpreload/target/release/mkpreload HOME=/nonexistent/first-build) ;;
        kernel) wrap=(setarch x86_64 --uname-2.6) ;;
        aslr) wrap=(setarch x86_64 -R) ;;
        num_cpus) wrap=(taskset -c 0) ;;
        time) envs=(LD_PRELOAD=$FT FAKETIME=+7d) ;;
        tsc-rate) [ "$sys" = triseal ] || continue
                  envs=(ENARX=$K/enarx-host ENARX_TEST_TSC_SCALE=3) ;;
        umask) wrap=(bash -c 'umask 0077; exec "$@"' umask) ;;
        locales) envs=(LANG=en_GB.utf8 LC_ALL=en_GB.utf8) ;;
        timezone) envs=(TZ=GMT+12) ;;
        exec_path) envs=(PATH=$PATH:/i_capture_the_path) ;;
        domain_host) wrap=(dock --hostname canary-host --user "$(id -u):$(id -g)" ::) ;;
        declared) [ "$sys" = triseal ] || continue
                  DECLX='TZ = "GMT+12"
LANG = "en_GB.utf8"
LC_ALL = "en_GB.utf8"
HOME = "/nonexistent/first-build"
CAPTURE_ENVIRONMENT = "i_capture_the_environment"' ;;
        *) echo "unknown variation $v"; exit 1 ;;
      esac
      if [ "${v%-control}" = fileordering ]; then
        [ "$wl" = canary ] || continue
        T=/dev/shm/canary-fo-src
        [ "$v" = fileordering ] && mktree "$K/proj" "$T" -r || mktree "$K/proj" "$T" ""
        envs+=(SRCDIR=$T)
        # The build directory on tmpfs too (the drivers copy the tree with
        # tar, in the order readdir gives it): on ext4 the order is the name
        # hashes', whatever the order of creation.
        W=/dev/shm/canary-fo-work-$sys
      fi
      build "$sys" "$wl" "$v" "$W" "${wrap[@]}" -- "${envs[@]}"
      # Files a container wrote under another uid: back to us, so that the
      # next build can remove them.
      [ "$v" = user_group ] && docker run --rm -v "$E:$E" canary-runner chown -R "$(id -u):$(id -g)" "$W"
      [ "$v" = build_path ] && rm -rf "$E/work-vary-elsewhere"
    done
  done
done
echo "end $(date)" >> "$R/env.txt"
python3 "$K/vary-report.py" "$R" | tee "$R/report.md"
echo "VARY_DONE $R"
