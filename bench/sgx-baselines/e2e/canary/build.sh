#!/bin/bash
# The canary benchmarks' TriSeal runtime: the clang evaluation's runtime
# (e2e/clang/build.sh: the runtime under evaluation + clang, ld.lld and the
# logical-clock grates in its allowlist) with one more module allowed, the
# canary project's build-time tool `probe` (canary/proj/probe.c, built with
# lind_compile -s in alice-test, precompiled here with explicit bounds
# checks). Nothing else changes; the MRENCLAVE is new.
#   canary/enarx-default  that allowlist
#   canary/enarx-host     + the test-only host knobs of eval/build.sh
#                         (ENARX_TEST_REFUSE_SPAWN, ENARX_TEST_TSC_SCALE)
# The ebc tree is rebuilt with its own allowlist afterwards and compared
# with ebc/enarx-default.
set -u
B=$HOME/sgx-bench; E=$B/e2e; X=$E/ebc; C=$E/clang; K=$E/canary
LB=$E/lw-ebc/src/lind-boot/target/release/lind-boot
export ENARX_MUSL_DIRECT_LIB=$E/ze/musl-direct
T=$X/TriSeal
mkdir -p $K/ebc $K/wasm
docker cp alice-test:/home/lind/canary/probe.wasm $K/wasm/probe.wasm
cp $K/wasm/probe.wasm $K/ebc/.pc-probe.wasm
$LB --precompile --explicit-bounds-checks $K/ebc/.pc-probe.wasm > $K/ebc/.pc-probe.log 2>&1 ||
  { echo "precompile probe FAILED"; tail -3 $K/ebc/.pc-probe.log; exit 1; }
mv $K/ebc/.pc-probe.cwasm $K/ebc/probe; chmod 755 $K/ebc/probe; rm -f $K/ebc/.pc-probe.*
{ cat $C/allowlist.txt
  echo "#"
  echo "# + the canary benchmarks (e2e/canary, 2026-11-02): probe, the canary project's"
  echo "# build-time tool (lind-wasm $(docker exec alice-test git -C /home/lind/lind-wasm log -1 --format=%h) sysroot, alice-test)."
  echo "$(sha256sum < $K/ebc/probe | cut -c1-64)  probe"; } > $K/allowlist.txt
tail -3 $K/allowlist.txt

F=$T/src/backend/sgx/enarxcall.rs
build() {  # <name>
  echo "== build $1 $(date +%T)"
  export ENARX_CWASM_ALLOWLIST=$K/allowlist.txt
  (cd $T && cargo build --release > $K/build-$1.log 2>&1); local rc=$?
  echo "build $1 EXIT=$rc $(date +%T)"; grep -E "^error" -A8 $K/build-$1.log | head -20
  [ $rc = 0 ] && cp $T/target/release/enarx $K/enarx-$1
}
build default
cp $F $F.orig
python3 - $F <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
anchor = "            if *addr != 0 {\n                keep.push_tcs(*addr);\n            }\n"
assert s.count(anchor) == 1
s = s.replace(anchor, "            if std::env::var_os(\"ENARX_TEST_REFUSE_SPAWN\").is_some() {\n                error!(\"test: refusing to start a thread\");\n                *ret = -EAGAIN as usize;\n                return Ok(None);\n            }\n" + anchor)
anchor = "            let cpuid_ret = unsafe { core::arch::x86_64::__cpuid_count(*leaf as _, *subleaf as _) };\n"
assert s.count(anchor) == 1
s = s.replace(anchor, anchor.replace("let cpuid_ret", "let mut cpuid_ret") +
  "            if *leaf == 0x15 {\n"
  "                if let Some(f) = std::env::var(\"ENARX_TEST_TSC_SCALE\").ok().and_then(|v| v.parse::<f64>().ok()) {\n"
  "                    let ecx = (cpuid_ret.ecx as f64 * f) as u32;\n"
  "                    error!(\"test: CPUID 0x15 crystal {} Hz reported as {} Hz\", cpuid_ret.ecx, ecx);\n"
  "                    cpuid_ret.ecx = ecx;\n"
  "                }\n"
  "            }\n")
open(p, "w").write(s)
PY
build host
mv $F.orig $F
unset ENARX_CWASM_ALLOWLIST
(cd $T && cargo build --release > $K/build-default-again.log 2>&1); echo "default again EXIT=$?"
cmp -s $T/target/release/enarx $X/enarx-default && echo "tree target == ebc/enarx-default" || echo "tree target differs from ebc/enarx-default"
sha256sum $K/enarx-* $K/ebc/probe | cut -c1-16
echo BUILD_DONE $(date +%T)
