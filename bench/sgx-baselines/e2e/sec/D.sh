#!/bin/bash
# Attack D: the host tampers with a build *input* that the system claims to
# protect, before the enclave consumes it.
#   triseal          flip a byte in the signed preload bundle in /dev/shm
#   gramine-trusted  flip a byte in a trusted_files toolchain binary (cc1)
#   gramine-encrypted  flip a ciphertext byte of a source on the encrypted
#                    mount, then swap in another valid protected file
#   gramine-allowed  flip a byte in a source file on the plain host mount
#   occlum           (see D-occlum.sh) flip a byte in the MAC'd SEFS build layer
# Oracle: PREVENT/DETECT/COMPROMISE as in lib.sh, plus ACCEPTED: the system
# read the tampered input without complaint and the build then failed.
#
# Usage: D.sh <triseal|gramine-trusted|gramine-encrypted|gramine-allowed|occlum>
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/lib.sh"
sys=${1:?system}

case "$sys" in
triseal)
  W=$SECE/work-sec-D-triseal
  # Control: a clean signed build must verify.
  bash "$HERE/../lua-triseal-shm.sh" "$W.clean" >"$RESULTS/D-triseal.clean.out" 2>&1
  okc=$(grep -c "attestations verified" "$RESULTS/D-triseal.clean.out")
  built=$(grep -c "Lua 5.5" "$RESULTS/D-triseal.clean.out")
  # Attack: flip a byte of the bundle (last source file) before the keep maps it.
  TAMPER=1 bash "$HERE/../lua-triseal-shm.sh" "$W" >"$RESULTS/D-triseal.out" 2>&1
  rc=$?
  tbuilt=$(grep -c "Lua 5.5" "$RESULTS/D-triseal.out")
  vfail=$(grep -c "ATTESTATION CHECK FAILED" "$RESULTS/D-triseal.out")
  if [ "$built" -ge 1 ] && [ "$okc" -ge 1 ] && [ "$tbuilt" -eq 0 ]; then
    sec_log D triseal bundle DETECT "clean built+verified; tampered bundle produced no lua (keep refused) rc=$rc"
  elif [ "$tbuilt" -ge 1 ] && [ "$vfail" -ge 1 ]; then
    sec_log D triseal bundle DETECT "tampered bundle built but attestation verify FAILED"
  elif [ "$tbuilt" -ge 1 ]; then
    sec_log D triseal bundle COMPROMISE "tampered bundle produced lua and verify passed"
  else
    sec_log D triseal bundle ERROR "clean_built=$built clean_ok=$okc tbuilt=$tbuilt rc=$rc"
  fi
  ;;

gramine-trusted)
  . "$HERE/gramine-stage.sh"
  W=$SECE/work-sec-D-trusted
  g_stage allowed "$W" || { sec_log D gramine trusted ERROR "stage"; exit 1; }
  # Control: with cc1 intact the build succeeds.
  g_make || { sec_log D gramine trusted ERROR "clean build rc=$? (cc1 intact)"; exit 1; }
  cbuilt=$(h "$(g_host lua)")
  # Tamper cc1, a trusted_files binary hashed at sign time and re-checked on
  # open. Confirm natively that the tampered cc1 still execs (so a failure
  # under Gramine is the hash check, not a broken binary).
  cc1=$E/native-gcc/bin/cc1; cp -p "$cc1" "$cc1.secbak"
  off=$((0x$(readelf -SW "$cc1" | sed -n 's/.*\] \.text *PROGBITS *[0-9a-f]* \([0-9a-f]*\).*/\1/p' | head -1)))
  flip_byte "$cc1" $((off + 4096))
  "$cc1" --version >/dev/null 2>&1; natrc=$?  # does the tampered cc1 still exec natively?
  rm -f "$(g_host lapi.o)" "$(g_host liblua.a)" "$(g_host lua)"
  g_make; rc=$?
  cp -p "$cc1.secbak" "$cc1"; rm -f "$cc1.secbak"
  # Gramine re-hashes a trusted_file at open, before any execution, so a
  # mismatch is refused regardless of whether the tamper also breaks the code.
  if [ "$cbuilt" != - ] && [ $rc -ne 0 ]; then
    sec_log D gramine trusted DETECT "clean build ok; tampered cc1 refused by Gramine (hash check at open), build failed at cc1 rc=$rc; native-exec rc=$natrc"
  elif [ $rc -eq 0 ]; then
    sec_log D gramine trusted COMPROMISE "tampered cc1 ran, build rc=0"
  else
    sec_log D gramine trusted ERROR "cbuilt=$cbuilt rc=$rc"
  fi
  ;;

gramine-encrypted)
  . "$HERE/gramine-stage.sh"
  W=$SECE/work-sec-D-encrypted
  g_stage encrypted "$W" || { sec_log D gramine encrypted ERROR "stage"; exit 1; }
  g_make || { sec_log D gramine encrypted ERROR "clean rc=$?"; exit 1; }
  r=$(h "$(g_plain lua)")
  [ "$r" = - ] && { sec_log D gramine encrypted ERROR "clean build made no lua"; exit 1; }
  # The host has the ciphertext but not the key. Two tampers of the protected
  # source lstate.c, each followed by a rebuild of what depends on it:
  #   flip  one ciphertext byte (integrity)
  #   swap  another valid protected file under the same key (lapi.c), so only
  #         a binding between a file's contents and its path can catch it
  # A failed build counts as DETECT only when Gramine reports the protected
  # file; a compile/link error after a swap means Gramine accepted the file.
  src=$(g_host lstate.c); cp -p "$src" "$src.secbak"
  pf_err() { grep -iE "lstate\.c.*(denied|invalid|integrity|corrupt)|protected|pf_|mac" "$W.log" | head -1; }
  rebuild() { rm -f "$(g_host lstate.o)" "$(g_host liblua.a)" "$(g_host lua)" "$(g_host luac)"; g_make; }
  for variant in flip swap; do
    cp -p "$src.secbak" "$src"
    case $variant in
      flip) flip_byte "$src" $(( $(stat -c %s "$src") / 2 )) ;;
      swap) cp "$(g_host lapi.c)" "$src"; touch -r "$src.secbak" "$src" ;;
    esac
    rebuild; rc=$?
    cp "$W.log" "$RESULTS/D-encrypted-$variant.log"
    n=$(h "$(g_plain lua)"); e=$(pf_err)
    if [ $rc -ne 0 ] && [ -n "$e" ]; then
      sec_log D gramine "encrypted-$variant" DETECT "rebuild rc=$rc; $e"
    elif [ $rc -ne 0 ]; then
      sec_log D gramine "encrypted-$variant" ACCEPTED "Gramine read the tampered file; build then failed rc=$rc: $(grep -iE 'error' "$W.log" | head -1)"
    elif [ "$n" != "$r" ]; then
      sec_log D gramine "encrypted-$variant" COMPROMISE "rebuild ok, lua $r->$n"
    else
      sec_log D gramine "encrypted-$variant" INEFFECTIVE "rebuild ok, lua unchanged"
    fi
  done
  cp -p "$src.secbak" "$src"; rm -f "$src.secbak"
  ;;

gramine-allowed)
  . "$HERE/gramine-stage.sh"
  W=$SECE/work-sec-D-allowed
  g_stage allowed "$W" || { sec_log D gramine allowed ERROR "stage"; exit 1; }
  g_make || { sec_log D gramine allowed ERROR "clean rc=$?"; exit 1; }
  r=$(h "$(g_plain lua)")
  # Tamper a source on the plain mount with a *compiling* change (a valid
  # declaration), so the change flows into the output rather than breaking
  # the compile; force a full rebuild.
  src=$(g_host lstate.c); { echo "int __sec_D_marker = 1;"; cat "$src"; } >"$src.n"; mv "$src.n" "$src"
  rm -f "$(g_host lstate.o)" "$(g_host liblua.a)" "$(g_host lua)" "$(g_host luac)"
  g_make; rc=$?
  n=$(h "$(g_plain lua)"); mk=$(nm "$(g_plain lua)" 2>/dev/null | grep -c __sec_D_marker)
  if [ $rc -ne 0 ]; then sec_log D gramine allowed ERROR "build aborted rc=$rc"
  elif [ "$n" = - ]; then sec_log D gramine allowed DETECT "no lua"
  elif [ "$n" != "$r" ] && [ "$mk" -ge 1 ]; then sec_log D gramine allowed COMPROMISE "injected source accepted (marker in lua), output changed, no check"
  else sec_log D gramine allowed INEFFECTIVE "unchanged mk=$mk"; fi
  ;;

occlum) "$HERE/D-occlum.sh";;
*) echo "unknown $sys" >&2; exit 2;;
esac
