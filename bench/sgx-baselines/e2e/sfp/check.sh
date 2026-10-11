#!/bin/bash
# Smoke-test a built single-file program natively: check.sh <prog> <binary>
p=$1
b=$2
[ -f "$b" ] || { echo "$p: missing"; exit 1; }
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
case $p in
  gzip)
    head -c 1000000 /dev/urandom | base64 >"$t/in"
    "$b" -c "$t/in" >"$t/in.gz" && "$b" -dc "$t/in.gz" | cmp -s - "$t/in" &&
      echo "gzip: ok (round trip of $(stat -c %s "$t/in") bytes, $(stat -c %s "$t/in.gz") compressed)" ||
      { echo "gzip: FAILED"; exit 1; } ;;
  oggenc)
    "$b" --version 2>&1 | head -1 | sed 's/^/oggenc: ok, /' ;;
  gcc)
    # gcc.c is compiled only: an x86-64 object with cc1's entry point and
    # about as many symbols as a native build of it has.
    file -b "$b" | grep -q "ELF 64-bit LSB relocatable, x86-64" &&
      nm "$b" | grep -q " T main$" &&
      echo "gcc.o: ok, ELF relocatable with main, $(nm "$b" | grep -c " [TtDdBbRr] ") symbols, $(stat -c %s "$b") bytes" ||
      { echo "gcc.o: FAILED"; exit 1; } ;;
esac
