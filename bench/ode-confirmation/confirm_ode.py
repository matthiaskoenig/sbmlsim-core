#!/usr/bin/env python3
"""Freeze or execute the randomized, process-replicated ODE confirmation experiment."""
import argparse
import hashlib
import json
import os
import random
import subprocess
from pathlib import Path

SEED = 29


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def freeze_or_check(path, value):
    content = json.dumps(value, sort_keys=True, indent=2) + '\n'
    if path.exists():
        if path.read_text() != content:
            raise ValueError(f'frozen input changed: {path}')
    else:
        path.write_text(content)


def plan(args):
    OUT = args.out.resolve()
    MATRIX = args.jobs.resolve()
    RUNTIMES = {'baseline': args.baseline_runtime.resolve(), 'feature': args.feature_runtime.resolve()}
    OUT.mkdir(parents=True, exist_ok=True)
    jobs = [j for j in json.loads(MATRIX.read_text()) if not j['native']]
    names = [j['name'] for j in jobs]
    if len(set(names)) != 24 or len(jobs) != 24:
        raise ValueError('expected exactly 24 distinct WASM jobs')
    assets = {str(MATRIX): digest(MATRIX), "cpu": args.cpu, "seed": SEED, "blocks": 3}
    for family, runtime in RUNTIMES.items():
        for fname in ['mathlib.wasm', 'engine.wasm']:
            path = runtime / fname
            assets[str(path)] = digest(path)
    for job in jobs:
        model_dir = MATRIX.parent / job['directory']
        for fname in ['model.wasm', 'model.json']:
            path = model_dir / fname
            assets[str(path)] = digest(path)
    for fname in ['confirm_ode.py', 'confirm_ode_stats.py', 'confirmation-protocol.md']:
        assets[fname] = digest(Path(__file__).with_name(fname))
    freeze_or_check(OUT / 'inputs.json', assets)
    rng = random.Random(SEED)
    ab = ['forward'] * 12 + ['reverse'] * 12
    aa = ['forward'] * 4 + ['reverse'] * 4
    rng.shuffle(ab)
    rng.shuffle(aa)
    trials = []
    for i in range(8):
        group = [('AB', ab[3*i+j]) for j in range(3)] + [('AA', aa[i])]
        rng.shuffle(group)
        for kind, orientation in group:
            order = list(names)
            rng.shuffle(order)
            index = len(trials) + 1
            trials.append(dict(index=index, kind=kind, orientation=orientation, cases=order))
    freeze_or_check(OUT / 'schedule.json', trials)
    return trials, {j['name']: j for j in jobs}


def run(args):
    OUT = args.out.resolve()
    MATRIX = args.jobs.resolve()
    RUNTIMES = {'baseline': args.baseline_runtime.resolve(), 'feature': args.feature_runtime.resolve()}
    if (OUT / 'driver.json').exists():
        raise ValueError('an experiment already started here; use a new output directory')
    trials, jobs = plan(args)
    binary = args.binary.resolve()
    freeze_or_check(OUT / 'driver.json', dict(binary=str(binary), sha256=digest(binary)))
    env = {**os.environ, 'SBMLSIM_ODE_BENCH_BLOCKS': '3',
           'SBMLSIM_ODE_BENCH_CACHE': str(args.cache.resolve() if args.cache else OUT / 'cache')}
    for trial in trials:
        prefix = OUT / f"trial-{trial['index']:02d}-{trial['kind']}-{trial['orientation']}"
        output = prefix.with_suffix('.json')
        if output.exists():
            raise ValueError(f'trial output already exists: {output}')
        order = [dict(jobs[name]) for name in trial['cases']]
        for j in order:
            j['directory'] = str(MATRIX.parent / j['directory'])
        jobfile = prefix.with_suffix('.jobs.json')
        freeze_or_check(jobfile, order)
        first = RUNTIMES['baseline']
        second = RUNTIMES['feature' if trial['kind'] == 'AB' else 'baseline']
        if trial['orientation'] == 'reverse':
            first, second = second, first
        partial = prefix.with_suffix('.partial')
        if partial.exists():
            raise ValueError(f'partial trial output already exists: {partial}')
        cmd = ['taskset', '-c', str(args.cpu), str(binary), 'paired', str(jobfile), str(first), str(second)]
        print(f"{trial['index']:02d}/32 {trial['kind']} {trial['orientation']}", flush=True)
        with partial.open('w') as out, prefix.with_suffix('.log').open('a') as log:
            subprocess.run(cmd, env=env, stdout=out, stderr=log, check=True)
        records = json.loads(partial.read_text())
        if [r['name'] for r in records] != trial['cases'] or any(
                len(r[k]) != 6 for r in records for k in ('baseline_samples_s','feature_samples_s')):
            raise ValueError(f'incomplete trial or driver ignored three-block setting: {prefix}')
        partial.replace(output)


if __name__ == '__main__':
    args = argparse.ArgumentParser()
    args.add_argument('action', choices=['plan','run'])
    args.add_argument('--binary', type=Path)
    args.add_argument('--jobs', required=True, type=Path)
    args.add_argument('--baseline-runtime', required=True, type=Path)
    args.add_argument('--feature-runtime', required=True, type=Path)
    args.add_argument('--out', required=True, type=Path)
    args.add_argument('--cache', type=Path)
    args.add_argument('--cpu', type=int, default=0)
    parsed = args.parse_args()
    if parsed.action == 'plan':
        trials, jobs = plan(parsed)
        print(f'Prespecified {len(trials)} processes and {len(jobs)} cases in {parsed.out}')
    else:
        if not parsed.binary:
            args.error('--binary is required for run')
        run(parsed)
