#!/bin/bash
# Both canary benchmarks, one after the other: canary/time.sh, then
# canary/vary.sh, into results/canary-<ts>/{time,vary}.
set -u
K=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
R=$HOME/sgx-bench/results/canary-$(date +%Y%m%d-%H%M%S); mkdir -p "$R"
( while :; do echo "$(date +%T) load $(cut -d' ' -f1-3 /proc/loadavg) | $(ps -eo user:12,pcpu,comm --sort=-pcpu --no-headers | head -4 | tr -s ' ' | tr '\n' ';')"; sleep 60; done ) >"$R/load.log" 2>&1 &
LOADER=$!
trap 'kill $LOADER 2>/dev/null' EXIT
cp -r "$HOME/sgx-bench/results/canary-classes" "$R/classes" 2>/dev/null
bash "$K/time.sh" "$R/time" > "$R/time.log" 2>&1; echo "time rc=$?" >> "$R/chain.log"
bash "$K/vary.sh" "$R/vary" > "$R/vary.log" 2>&1; echo "vary rc=$?" >> "$R/chain.log"
echo "CHAIN_DONE $R" >> "$R/chain.log"
