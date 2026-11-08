#!/usr/bin/env python3
"""Compare engines' CPU time on the benchmarks in this directory.

    python3 tools/bench/bench.py [--runs N] [--filter SUBSTR] BIN...

Runs every `*.php` here (or those whose name contains SUBSTR) under each
BIN with `-n`, interleaved, N times (default 5), and prints the minimum and
median CPU time (user + sys, from RUSAGE_CHILDREN). On macOS it also prints
the minimum instructions retired (`/usr/bin/time -l`), which load on the
machine barely moves: compare those first. Every engine must print the
same output, or the run stops.
"""

import argparse
import os
import re
import resource
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(binary, script, count):
    cmd = [binary, "-n", str(script)]
    if count:
        cmd = ["/usr/bin/time", "-l"] + cmd
    r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
    p = subprocess.run(cmd, capture_output=True)
    r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
    if p.returncode != 0:
        sys.exit(f"{binary} {script.name}: exit {p.returncode}\n{p.stderr.decode(errors='replace')}")
    cpu = r1.ru_utime - r0.ru_utime + r1.ru_stime - r0.ru_stime
    ins = None
    if count:
        m = re.search(rb"(\d+)\s+instructions retired", p.stderr)
        ins = int(m.group(1)) if m else None
    return cpu, ins, p.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--filter", default="")
    ap.add_argument("bins", nargs="+")
    a = ap.parse_args()
    count = sys.platform == "darwin"
    scripts = sorted(p for p in HERE.glob("*.php") if a.filter in p.name)
    cpu = {}
    ins = {}
    for _ in range(a.runs):
        for s in scripts:
            out = None
            for b in a.bins:
                c, i, o = run(b, s, count)
                if out is not None and o != out:
                    sys.exit(f"{s.name}: {b} prints something else")
                out = o
                cpu.setdefault((b, s), []).append(c)
                if i is not None:
                    ins.setdefault((b, s), []).append(i)
    names = [os.path.basename(os.path.dirname(os.path.dirname(b))) or b for b in a.bins]
    head = "%-14s" % "bench"
    for n in names:
        head += "%32s" % n[-30:]
    print(head)
    for s in scripts:
        row = "%-14s" % s.name
        for b in a.bins:
            v = cpu[(b, s)]
            cell = "%.3f/%.3fs" % (min(v), statistics.median(v))
            if (b, s) in ins:
                cell += " %.3fG" % (min(ins[(b, s)]) / 1e9)
            row += "%32s" % cell
        print(row)
    print(f"(CPU seconds min/median, instructions retired min; {a.runs} interleaved runs)")


if __name__ == "__main__":
    main()
