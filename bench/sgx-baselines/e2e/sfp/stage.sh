#!/bin/bash
# Fetch the single-file programs and lay out one tree per program: the
# source and a one-rule Makefile, so that each is built by make + gcc like
# the Lua build. Records the sources' SHA-256 (the page publishes none).
# Usage: e2e/sfp/stage.sh
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sfp.conf"
URL=https://people.csail.mit.edu/smcc/projects/single-file-programs
mkdir -p "$SFP/src"
cd "$SFP/src"
for f in gzip.c oggenc.c gcc.c.bz2; do
  [ -f "$f" ] || curl -sSfLO "$URL/$f"
done
[ -f gcc.c ] || bunzip2 -k gcc.c.bz2
sha256sum gzip.c oggenc.c gcc.c.bz2 gcc.c >SHA256SUMS
ls -l gzip.c oggenc.c gcc.c.bz2 gcc.c
for p in "${SFP_PROGS[@]}"; do
  t=$SFP/tree/$p
  rm -rf "$t" && mkdir -p "$t"
  cp "$SFP/src/$p.c" "$t/"
  mkdir -p "$t/compat/sys"
  printf '/* Empty: <sys/sysctl.h> left glibc in 2.32. gcc.c includes it but on\n   Linux uses nothing from it (only the BSD HW_PHYSMEM, HW_USERMEM). */\n' \
    >"$t/compat/sys/sysctl.h"
  o=$(sfp_out "$p")
  if [ "$o" = "$p.o" ]; then
    printf '# CC and CFLAGS come from the command line (compiled only).\n%s: %s.c\n\t$(CC) $(CFLAGS) -c -o %s %s.c\n' \
      "$o" "$p" "$o" "$p" >"$t/Makefile"
  else
    printf '# CC, CFLAGS and LIBS come from the command line.\n%s: %s.c\n\t$(CC) $(CFLAGS) -o %s %s.c $(LIBS)\n' \
      "$o" "$p" "$o" "$p" >"$t/Makefile"
  fi
done
echo "trees: $SFP/tree/{${SFP_PROGS[*]}}"
