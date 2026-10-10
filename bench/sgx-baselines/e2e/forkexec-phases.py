"""Per-cage fork/exec phase durations from a timestamped TriSeal log."""
import re, sys, statistics
from collections import defaultdict

marks = [  # (phase-end name, regex with cage group)
    ("fork_start", r"fork_call entered, parent_cage=\d+, child_cage=(\d+)"),
    ("thread", r"fork spawned thread entered, child_cage=(\d+)"),
    ("vmmap_begin", None),
    ("mem_copied", r"forked memory from parent cage \d+ to child cage (\d+)"),
    ("fork_done", r"fork instantiate_with_lind \(child\) returned, child_cage=(\d+)"),
    ("exec_call", r"execve cage=(\d+) opening exec file"),
    ("opened", r"exec-3i open cage=(\d+)"),
    ("read_done", r"execve cage=(\d+) read \d+ bytes"),
    ("deser_done", r"execve cage=(\d+) module deserialize"),
    ("rewound", r"execve on_called fired, cage=(\d+)"),
    ("main", r"start executing main module in cage (\d+)"),
]
t = defaultdict(dict)
prog = {}
last_child = None
for line in open(sys.argv[1], errors="replace"):
    m = re.match(r"(\d+\.\d+) (.*)", line)
    if not m:
        continue
    ts, msg = float(m.group(1)), m.group(2)
    if "[fork_vmmap] begin" in msg and last_child:
        t[last_child].setdefault("vmmap_begin", ts)
    pm = re.search(r"execve cage=(\d+) opening exec file (\S+)", msg)
    if pm:
        prog[pm.group(1)] = pm.group(2).rsplit("/", 1)[-1]
    for name, rx in marks:
        if rx and (mm := re.search(rx, msg)):
            t[mm.group(1)].setdefault(name, ts)
            if name == "fork_start":
                last_child = mm.group(1)

phases = [("fork: start->thread", "fork_start", "thread"),
          ("fork: copy memory", "vmmap_begin", "mem_copied"),
          ("fork: total", "fork_start", "fork_done"),
          ("child runs to execve", "fork_done", "exec_call"),
          ("exec: open via 3i", "exec_call", "opened"),
          ("exec: read module via 3i", "opened", "read_done"),
          ("exec: deserialize", "read_done", "deser_done"),
          ("exec: asyncify unwind/rewind", "deser_done", "rewound"),
          ("exec: instantiate -> main", "rewound", "main"),
          ("fork start -> new main", "fork_start", "main")]
by_prog = defaultdict(lambda: defaultdict(list))
for cage, d in t.items():
    p = prog.get(cage)
    if not p:
        continue
    for label, a, b in phases:
        if a in d and b in d:
            by_prog[p][label].append(d[b] - d[a])
for p, ph in by_prog.items():
    n = len(ph["fork start -> new main"])
    print(f"\n{p} (n={n}) mean seconds:")
    for label, *_ in phases:
        v = ph.get(label)
        if v:
            print(f"  {label:32s} {statistics.mean(v):7.3f}   (min {min(v):.3f} max {max(v):.3f})")
