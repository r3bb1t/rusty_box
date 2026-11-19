#!/usr/bin/env python3
"""Interleave original, previous and patched perfbench, plus both memory maps.

Build all binaries using the same release/std configuration. Pass the current
libtest executable for the two memory_environment_perf cases. Every sample
is pinned to the same allowed host CPU; construction is excluded by the guest
benchmark itself. All samples, binary hashes and output are retained.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
for name in ('original', 'previous', 'patched', 'test-binary', 'output'):
    parser.add_argument('--' + name, required=True, type=Path)
parser.add_argument('--instructions', type=int, default=200_000_000)
parser.add_argument('--rounds', type=int, default=7)
parser.add_argument('--modes', nargs='+', choices=('mixed', 'alu', 'string'), default=['mixed', 'alu', 'string'])
parser.add_argument('--full-system-only', action='store_true')
args = parser.parse_args()
if min(args.instructions, args.rounds) <= 0:
    parser.error('instructions and rounds must be positive')
paths = {name: str(getattr(args, name.replace('-', '_')).resolve())
         for name in ('original', 'previous', 'patched', 'test-binary')}
cpu = min(os.sched_getaffinity(0))
report = {'executables': paths, 'sha256': {k: hashlib.sha256(Path(v).read_bytes()).hexdigest() for k, v in paths.items()},
          'affinity_cpu': cpu, 'instructions': args.instructions, 'rounds': args.rounds, 'samples': [], 'summary': {}}
for mode in args.modes:
    versions = ['original', 'previous', 'patched']
    if not args.full_system_only:
        versions += ['memory_pc', 'memory_flat', 'memory_flat_unaligned']
    for round_index in range(args.rounds):
        order = versions[round_index % len(versions):] + versions[:round_index % len(versions)]
        if round_index % 2:
            order = order[::-1]
        for version in order:
            env = dict(os.environ, PERFBENCH_MODE=mode, PERFBENCH_INSN=str(args.instructions), PERFBENCH_CPUS='1',
                       MEMBENCH_MODE=mode, MEMBENCH_INSN=str(args.instructions), MEMBENCH_BACKEND='flat' if version.startswith('memory_flat') else 'pc')
            env.pop('MEMBENCH_ALIGNED', None)
            if version == 'memory_flat':
                env['MEMBENCH_ALIGNED'] = '1'
            if version.startswith('memory_'):
                command = [paths['test-binary'], 'cpu::memory_env_tests::memory_environment_perf', '--exact', '--ignored', '--nocapture']
                pattern = r'MEMBENCH backend=\w+ mode=\w+ instructions=(\d+) ns=([\d.]+)'
            else:
                command = [paths[version]]
                pattern = r'(\d+) insns in [\d.]+s = [\d.]+ MIPS \(([\d.]+) ns/insn\)'
            before_cpu = resource.getrusage(resource.RUSAGE_CHILDREN)
            result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True,
                                    preexec_fn=lambda: os.sched_setaffinity(0, {cpu}))
            after_cpu = resource.getrusage(resource.RUSAGE_CHILDREN)
            match = re.search(pattern, result.stdout)
            if match is None or int(match[1]) != args.instructions:
                raise RuntimeError(result.stdout)
            cpu_seconds = after_cpu.ru_utime + after_cpu.ru_stime - before_cpu.ru_utime - before_cpu.ru_stime
            report['samples'].append({'mode': mode, 'round': round_index, 'version': version, 'ns': float(match[2]),
                                      'whole_process_cpu_ns_per_count': cpu_seconds * 1e9 / args.instructions,
                                      'output': result.stdout})
            args.output.write_text(json.dumps(report, indent=2) + '\n')
        print(f'{mode} round {round_index + 1}/{args.rounds}', flush=True)
    summary = {}
    for version in versions:
        values = [x['ns'] for x in report['samples'] if x['mode'] == mode and x['version'] == version]
        summary[version] = {'median_ns': statistics.median(values), 'min_ns': min(values), 'max_ns': max(values)}
    for old in ('original', 'previous'):
        summary['patched_vs_' + old + '_percent'] = (summary['patched']['median_ns'] / summary[old]['median_ns'] - 1) * 100
    if not args.full_system_only:
        summary['flat_vs_pc_percent'] = (summary['memory_flat']['median_ns'] / summary['memory_pc']['median_ns'] - 1) * 100
        summary['unaligned_flat_vs_pc_percent'] = (summary['memory_flat_unaligned']['median_ns'] / summary['memory_pc']['median_ns'] - 1) * 100
    report['summary'][mode] = summary
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(mode, json.dumps(summary), flush=True)
