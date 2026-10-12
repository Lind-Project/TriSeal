#!/bin/bash
# Build and run the Occlum experiment-E probe inside the Occlum container.
# Compiles probe-occlum.c natively (occlum-gcc), stages it + busybox into a
# fresh Occlum instance with /host (hostfs) and /tmp (ramfs), and runs it.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
I=$HERE/inst-e
GCC=/usr/local/occlum/bin/occlum-gcc

"$GCC" -O2 -o "$HERE/probe-occlum" "$HERE/probe-occlum.c"

rm -rf "$I"
occlum new "$I" >/dev/null
cd "$I"
cat >"$HERE/e.yaml" <<YAML
includes:
  - base.yaml
targets:
  - target: /bin
    copy:
      - files:
        - $HERE/probe-occlum
        - /opt/occlum/toolchains/busybox/glibc/busybox
YAML
rm -rf image
copy_bom -f "$HERE/e.yaml" --root image --include-dir /opt/occlum/etc/template >copy_bom.log 2>&1
jq '.resource_limits.user_space_size = "256MB"
  | .resource_limits.max_num_of_threads = 16
  | .process.default_stack_size = "8MB"
  | .env.default = ((.env.default // []) + ["PATH=/bin"])
  | .mount = ([.mount[] | select(.target != "/tmp" and .target != "/host")]
      + [{"target": "/tmp", "type": "ramfs"}, {"target": "/host", "type": "hostfs", "source": "./host"}])' \
  Occlum.json >o.json && mv o.json Occlum.json
mkdir -p host
occlum build >build.log 2>&1
echo "=== occlum run probe-occlum parent ==="
occlum run /bin/probe-occlum parent 2>&1
echo "=== run rc=$? ==="
