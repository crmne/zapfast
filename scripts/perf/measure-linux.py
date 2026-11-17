#!/usr/bin/env python3
"""Measure ZapFast's Linux process tree using /proc only."""
import argparse
import os
import time
from pathlib import Path

PROC = Path("/proc")
CLK_TCK = os.sysconf("SC_CLK_TCK")
CORES = os.cpu_count() or 1


def read_stat(pid):
    try:
        raw = (PROC / str(pid) / "stat").read_text()
        fields = raw[raw.rfind(")") + 2:].split()
        return int(fields[1]), int(fields[11]) + int(fields[12])
    except (OSError, ValueError, IndexError):
        return None


def tree(root):
    children = {}
    for entry in PROC.iterdir():
        if entry.name.isdigit():
            pid = int(entry.name)
            info = read_stat(pid)
            if info is not None:
                children.setdefault(info[0], []).append(pid)
    result, pending, seen = [], [root], set()
    while pending:
        pid = pending.pop()
        if pid in seen or read_stat(pid) is None:
            continue
        seen.add(pid)
        result.append(pid)
        pending.extend(children.get(pid, []))
    return result


def snapshot(root):
    pids = tree(root)
    count = pss = private = cpu = 0
    for pid in pids:
        info = read_stat(pid)
        if info is None:
            continue
        count += 1
        cpu += info[1]
        try:
            lines = (PROC / str(pid) / "smaps_rollup").read_text().splitlines()
        except OSError:
            continue
        for line in lines:
            key, sep, value = line.partition(":")
            if not sep:
                continue
            parts = value.split()
            if not parts or not parts[0].isdigit():
                continue
            amount = int(parts[0])
            if key == "Pss":
                pss += amount
            elif key in ("Private_Clean", "Private_Dirty"):
                private += amount
    return count, pss, private, cpu


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--interval", type=float, default=10.0)
    args = parser.parse_args()
    if min(args.pid, args.samples, args.interval) <= 0:
        parser.error("PID, samples and interval must be positive")
    if read_stat(args.pid) is None:
        parser.error("PID is not readable")

    print("ZapFast process-tree benchmark")
    print(f"Root PID: {args.pid}; samples: {args.samples}; interval: {args.interval}s")
    print(f"{'Sample':>6} {'PIDs':>6} {'PSS MiB':>12} {'Private MiB':>14} {'CPU %/core':>12} {'CPU %/system':>14}")
    for sample in range(1, args.samples + 1):
        before = snapshot(args.pid)
        time.sleep(args.interval)
        after = snapshot(args.pid)
        delta = max(0, after[3] - before[3])
        one_core = delta / CLK_TCK / args.interval * 100
        system = one_core / CORES
        print(f"{sample:>6} {after[0]:>6} {after[1]/1024:>12.2f} {after[2]/1024:>14.2f} {one_core:>12.3f} {system:>14.3f}")
    print("CPU is an interval average; one-core usage can exceed 100%.")


if __name__ == "__main__":
    main()
