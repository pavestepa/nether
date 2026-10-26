#!/usr/bin/env python3
"""Paired host benchmarks; no dependencies and no timing thresholds."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
BASE = ROOT / 'performance'


def command(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, **kwargs)


def checked(args, **kwargs):
    result = command(args, **kwargs)
    if result.returncode:
        raise RuntimeError(f'{args}: {result.stdout}\n{result.stderr}')
    return result


def measure(args, build=False):
    start = time.perf_counter_ns()
    result = command(args)
    seconds = (time.perf_counter_ns() - start) / 1e9
    if (build and result.returncode) or result.returncode < 0 or result.stderr:
        raise RuntimeError(f'{args}: exit={result.returncode}\n{result.stderr}')
    return seconds, result.returncode


def stats(values):
    return dict(median=statistics.median(values), minimum=min(values), maximum=max(values))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('runs', nargs='?', type=int, default=7)
    parser.add_argument('--build-runs', type=int, default=3)
    args = parser.parse_args()
    if args.runs < 3 or args.build_runs < 1:
        parser.error('use at least 3 execution runs and 1 build run')
    out = BASE / 'out'
    out.mkdir(exist_ok=True)
    rustc = os.environ.get('NETHER_RUSTC', 'rustc')
    checked(['cargo', 'build', '--release', '-p', 'nether', '--locked', '--offline'])
    nether = ROOT / 'target/release/nether'
    digest = hashlib.sha256()
    for folder in ['compiler', 'runtime', 'cli']:
        for path in sorted((ROOT / folder).rglob('*')):
            if path.is_file() and path.suffix in {'.rs', '.toml'}:
                digest.update(str(path.relative_to(ROOT)).encode())
                digest.update(path.read_bytes())
    cpu = platform.processor()
    if platform.system() == 'Darwin':
        probe = command(['sysctl', '-n', 'machdep.cpu.brand_string'])
        cpu = probe.stdout.strip() if probe.returncode == 0 else 'unavailable (sandbox)'
    data = dict(date=datetime.datetime.now().astimezone().isoformat(),
                platform=platform.platform(), cpu=cpu,
                rustc=checked([rustc, '--version']).stdout.strip(),
                commit=checked(['git', 'rev-parse', 'HEAD']).stdout.strip(),
                dirty=bool(checked(['git', 'status', '--porcelain']).stdout),
                compiler_source_sha256=digest.hexdigest(),
                runs=args.runs, build_runs=args.build_runs, warmups=1,
                methodology='Wall time including process startup; alternating paired order; median; overflow off; opt-level=3; host default CPU; no LTO',
                benchmarks=[])
    for source in sorted((BASE / 'bench').glob('*.nr')):
        name = source.stem
        rust_source = source.with_suffix('.rs')
        binaries = {lang: out / f'{name}-{lang}' for lang in ['rust', 'nether']}
        commands = {
            'rust': [rustc, '--edition=2021', '-C', 'opt-level=3', '-C', 'debuginfo=0', '-C', 'overflow-checks=off', str(rust_source), '-o', str(binaries['rust'])],
            'nether': [str(nether), 'build', str(source), '--release', '--overflow-checks=off', '-o', str(binaries['nether'])],
        }
        builds = {lang: [] for lang in binaries}
        for i in range(args.build_runs):
            for lang in (['rust', 'nether'] if i % 2 == 0 else ['nether', 'rust']):
                builds[lang].append(measure(commands[lang], build=True)[0])
        codes = {lang: measure([str(binary)])[1] for lang, binary in binaries.items()}
        if len(set(codes.values())) != 1:
            raise RuntimeError(f'{name}: result mismatch {codes}')
        samples = {lang: [] for lang in binaries}
        for i in range(args.runs):
            for lang in (['rust', 'nether'] if i % 2 == 0 else ['nether', 'rust']):
                elapsed, code = measure([str(binaries[lang])])
                if code != codes[lang]:
                    raise RuntimeError(f'{name}: unstable result {lang}={code}')
                samples[lang].append(elapsed)
        entry = dict(name=name, exit_code=codes['rust'],
                     source_sha256={p.suffix: hashlib.sha256(p.read_bytes()).hexdigest() for p in [source, rust_source]},
                     samples_s=samples, build_samples_s=builds,
                     runtime_s={lang: stats(v) for lang, v in samples.items()},
                     build_s={lang: stats(v) for lang, v in builds.items()},
                     binary_bytes={lang: p.stat().st_size for lang, p in binaries.items()})
        entry['ratio'] = entry['runtime_s']['nether']['median'] / entry['runtime_s']['rust']['median']
        data['benchmarks'].append(entry)
        print(f"{name:24} Rust={entry['runtime_s']['rust']['median']:.4f}s Nether={entry['runtime_s']['nether']['median']:.4f}s ratio={entry['ratio']:.3f}x", flush=True)
        (out / 'results.json').write_text(json.dumps(data, indent=2) + '\n')
    print(f'Raw measurements: {out / "results.json"}')


if __name__ == '__main__':
    main()
