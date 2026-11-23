#!/usr/bin/env python3
"""Compare equivalent release x86 executables on one Linux logical CPU."""

import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess


MODES = ("dense", "mapped", "pc-alu", "pc-mixed", "pc-string")
BORROWED = re.compile(r"borrowed-memory: (\d+) counts .*?\(([\d.]+) ns/count\)")
PC = re.compile(r"perfbench\[\w+\]: CountExhausted \| (\d+) insns .*?\(([\d.]+) ns/insn\)")


def measure(mode, count, borrowed, pc):
    env = dict(os.environ, RUST_MIN_STACK="16777216")
    if mode.startswith("pc-"):
        env.update(PERFBENCH_MODE=mode[3:], PERFBENCH_INSN=str(count), PERFBENCH_CPUS="1")
        command = [pc]
        pattern = PC
    else:
        env.update(
            BORROWED_BENCH_MAP=mode,
            BORROWED_BENCH_COUNT=str(count),
            BORROWED_BENCH_CONTROLLED="1",
        )
        command = [borrowed, "timed_borrowed_memory", "--exact", "--ignored", "--nocapture"]
        pattern = BORROWED
    result = subprocess.run(command, env=env, check=True, text=True, stdout=subprocess.PIPE)
    match = pattern.search(result.stdout)
    if match is None or int(match[1]) != count:
        raise RuntimeError(f"{command} returned unexpected output:\n{result.stdout}")
    return float(match[2])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--revision", nargs=3, action="append", required=True,
        metavar=("LABEL", "EXTERNAL_MEMORY_BINARY", "PERFBENCH_BINARY"),
    )
    parser.add_argument("--cpu", type=int, default=min(os.sched_getaffinity(0)))
    parser.add_argument("--rounds", type=int, default=7)
    parser.add_argument("--counts", type=int, default=80_000_000)
    parser.add_argument("--workloads", choices=MODES, nargs="+", default=MODES)
    parser.add_argument("--json", action="store_true", help="print the raw samples as JSON")
    args = parser.parse_args()
    if args.cpu not in os.sched_getaffinity(0):
        parser.error(f"CPU {args.cpu} is outside the allowed affinity")
    if args.rounds < 1 or args.counts < 1:
        parser.error("rounds and counts must be positive")
    revisions = {
        label: (Path(borrowed).resolve(strict=True), Path(pc).resolve(strict=True))
        for label, borrowed, pc in args.revision
    }
    if len(revisions) != len(args.revision):
        parser.error("revision labels must be unique")
    os.sched_setaffinity(0, {args.cpu})
    labels = list(revisions)
    samples = {}
    for mode in args.workloads:
        for borrowed, pc in revisions.values():
            measure(mode, min(args.counts, 2_000_000), borrowed, pc)
        samples[mode] = {label: [] for label in labels}
        for round_index in range(args.rounds):
            offset = round_index % len(labels)
            for label in labels[offset:] + labels[:offset]:
                samples[mode][label].append(measure(mode, args.counts, *revisions[label]))
        if not args.json:
            for label, values in samples[mode].items():
                print(
                    f"{mode:10} {label:12} {statistics.median(values):6.3f} ns/count "
                    f"[{min(values):.3f}..{max(values):.3f}] {values}",
                    flush=True,
                )
    if args.json:
        print(json.dumps({"cpu": args.cpu, "counts": args.counts, "samples": samples}, indent=2))


if __name__ == "__main__":
    main()
