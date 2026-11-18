#!/usr/bin/env python3
"""Interleave two prebuilt perfbench binaries; retain every sample and output."""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--baseline', required=True, type=Path)
parser.add_argument('--patched', required=True, type=Path)
parser.add_argument('--output', required=True, type=Path)
parser.add_argument('--instructions', type=int, default=200_000_000)
parser.add_argument('--rounds', type=int, default=7)
parser.add_argument('--pin', action='store_true', help='Use the first allowed host CPU for both binaries')
args = parser.parse_args()
if args.instructions <= 0 or args.rounds <= 0:
    parser.error('instructions and rounds must be positive')
executables = {'baseline': str(args.baseline.resolve()), 'patch': str(args.patched.resolve())}
cpu = min(os.sched_getaffinity(0)) if args.pin else None
samples = []
report = {'executables': executables, 'affinity_cpu': cpu, 'instructions_per_sample': args.instructions,
          'rounds': args.rounds, 'samples': samples, 'summary': {}}
pattern = re.compile(r'(\d+) insns in ([\d.]+)s = ([\d.]+) MIPS \(([\d.]+) ns/insn\)')
for mode in ['mixed', 'alu', 'string']:
    for round_index in range(args.rounds):
        order = ['baseline', 'patch'] if round_index % 2 == 0 else ['patch', 'baseline']
        for version in order:
            env = dict(os.environ, PERFBENCH_MODE=mode, PERFBENCH_INSN=str(args.instructions), PERFBENCH_CPUS='1')
            result = subprocess.run([executables[version]], env=env, text=True, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, check=True,
                                    preexec_fn=(lambda: os.sched_setaffinity(0, {cpu})) if cpu is not None else None)
            match = pattern.search(result.stdout)
            if match is None or int(match[1]) != args.instructions:
                raise RuntimeError(result.stdout)
            samples.append({'mode': mode, 'round': round_index, 'version': version,
                            'instructions': int(match[1]), 'seconds': float(match[2]),
                            'mips': float(match[3]), 'ns': float(match[4]), 'output': result.stdout})
            args.output.write_text(json.dumps(report, indent=2) + '\n')
    values = {version: [r['ns'] for r in samples if r['mode'] == mode and r['version'] == version]
              for version in executables}
    summary = {version: {'median': statistics.median(v), 'min': min(v), 'max': max(v)}
               for version, v in values.items()}
    summary['change_percent'] = (summary['patch']['median'] / summary['baseline']['median'] - 1) * 100
    report['summary'][mode] = summary
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(mode, json.dumps(summary), flush=True)
