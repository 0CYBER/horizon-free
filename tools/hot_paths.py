#!/usr/bin/env python3
"""Hot-path performance trends (docs/hot-path-trends.md): measure, record, and draw the page.

    python3 tools/hot_paths.py measure --game target/release/horizon-hot-paths-game --resources RES --out run.json
    python3 tools/hot_paths.py record run.json HISTORY     # validate, keep only known fields, add to HISTORY/runs
    python3 tools/hot_paths.py page HISTORY                # write HISTORY/index.html and HISTORY/README.md

`measure` runs the game's built-in benchmark (`HORIZON_HOT_PATHS`, crates/horizon-client/src/game/runtime/
hot_paths/; the executable is crates/horizon-hot-paths-game) on fixed, seeded workloads. Each workload is one measured region, and three numbers come out of it,
each divided by the workload's operations:

- instructions: Callgrind's count of the instructions the region executed (`Ir`), on every thread. It does not
  depend on how fast or how busy the machine is, so it is the number to read a trend from.
- allocations: the calls into the allocator the region made, read from the same Callgrind run.
- wall_ns: wall time of the region in runs without Valgrind. It depends on the machine and on what else runs
  there, so it is only compared between runs on one kind of machine.

Every number is taken several times, each in a fresh process; a run's result keeps every sample, and the page
shows the spread. Nothing here fails on a slow result: the trend is for people to read.

A result holds the commit, the date, a label for the kind of machine, its CPU model, the toolchain and the numbers. It never holds a path,
a file name, an asset name or anything read from the game's data.
"""
import argparse
import datetime
import html
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

SCHEMA = 1
# Callgrind names the allocator's entry points; the game allocates through mimalloc's aligned ones.
ALLOCATOR_ENTRIES = re.compile(r'^(mi_(m|z|c|re)alloc(_aligned)?|mi_(heap_)?(m|z|c|re)alloc\w*|malloc|calloc|realloc)$')
METRICS = {
    'instructions': 'instructions per operation (Callgrind Ir)',
    'allocations': 'allocator calls per operation',
    'wall_ns': 'nanoseconds per operation, wall clock',
}
NAME = re.compile(r'^[a-z0-9_.]{1,80}$')
# A step is flagged when a counted metric moves by more than this share and by more than three times its noise.
STEP = 0.01
# The share of the mean two samples may differ by before a metric is called noisy.
NOISY = 0.01


def scratch_env(root, **extra):
    """The game's environment for one run: lab.game_env with its homes under `root`, and no telemetry."""
    import lab  # here, so the record and page commands need nothing beside this file
    homes = {}
    for key in ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
        folder = Path(root) / key.lower()
        folder.mkdir(parents=True, exist_ok=True)
        homes[key] = folder
    # The worker pools' size follows the machine's cores unless it is fixed.
    return lab.game_env(HORIZON_TELEMETRY_URL='off', HORIZON_WORKER_THREADS='2', **homes, **extra)


# The native mod built into horizon-hot-paths-game: its package beside the executable, as the launcher leaves one.
LISTENER = 'hot_paths_listener'
LISTENER_MANIFEST = f"""id = '{LISTENER}'
name = '{LISTENER}'
version = '1.0.0'
api = '0.2'
side = 'both'
capabilities = ['registries']
[native]
game = '26.3'
horizon = '{'a' * 40}'
mixins = '{'b' * 40}'
"""


def install(game, folder):
    """Copy the benchmark's executable into `folder` with the listener mod's package beside it."""
    package = folder / 'horizon-native-mods' / LISTENER
    (package / 'src').mkdir(parents=True)
    (package / 'horizon-mod.toml').write_text(LISTENER_MANIFEST)
    (package / 'Cargo.toml').write_text(f"[package]\nname = '{LISTENER}'\nversion = '1.0.0'\n")
    (package / 'src/lib.rs').write_text('// Built into the executable.\n')
    return shutil.copy2(game, folder / 'horizon-client')


# The game wants a main-thread stack of 136 MiB and, finding less, raises the limit and starts itself again.
# Under Valgrind that second start would not see the raised limit, so every run gets the stack up front.
STACK = 160 << 20


def big_stack():
    import resource  # Unix only, and only a measuring run needs it
    hard = resource.getrlimit(resource.RLIMIT_STACK)[1]
    resource.setrlimit(resource.RLIMIT_STACK, (STACK if hard == resource.RLIM_INFINITY else min(STACK, hard), hard))


def run_game(game, resources, work, mods, only, prefix=()):
    """One process of the benchmark; returns its lines by workload name."""
    work.mkdir(parents=True)
    out = work / 'workloads.jsonl'
    env = scratch_env(work, HORIZON_HOT_PATHS=str(out), HORIZON_HOT_PATHS_MODS=mods)
    if only:
        env['HORIZON_HOT_PATHS_ONLY'] = only
    game_dir = work / 'game'
    game_dir.mkdir()
    command = [*prefix, str(game), '--resources', str(resources), '--game-dir', str(game_dir)]
    with open(work / 'game.log', 'w') as log:
        done = subprocess.run(command, cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT, preexec_fn=big_stack)
    if done.returncode != 0:
        tail = (work / 'game.log').read_text(errors='replace')[-3000:]
        ran = out.read_text() if out.exists() else ''
        raise SystemExit(f'the benchmark exited {done.returncode} ({mods}):\n{tail}\nworkloads finished:\n{ran}')
    return {row['name']: row for row in map(json.loads, out.read_text().splitlines())}


def parse_dump(text):
    """One Callgrind dump: (the client request's name, instructions, allocator calls).

    The dump lists, per function, the calls it made: a `cfn=` line naming the callee (its name the first time,
    then only its number), then `calls=COUNT ...`. The allocator calls are the calls into its entry points
    from any function that is not itself one of them (an entry point may call another)."""
    name = instructions = None
    entry = {}
    allocations = 0
    inside = False  # whether the current `fn=` is an allocator entry point
    callee = None
    for line in text.splitlines():
        if line.startswith('desc: Trigger:'):
            found = re.search(r'Client Request: (\S+)', line)
            name = found.group(1) if found else None
        elif line.startswith('totals:') or (line.startswith('summary:') and instructions is None):
            instructions = int(line.split()[1])
        elif line.startswith(('fn=', 'cfn=')):
            key, _, rest = line.partition('=')
            found = re.match(r'\((\d+)\)(?: (.*))?$', rest)
            if found:
                number, text_name = found.groups()
                if text_name is not None:
                    # Callgrind may append the object or a clone suffix; the symbol is the first word.
                    entry[number] = bool(ALLOCATOR_ENTRIES.match(text_name.split("'")[0].split()[0]))
                is_entry = entry.get(number, False)
            else:
                is_entry = bool(ALLOCATOR_ENTRIES.match(rest.split()[0])) if rest else False
            if key == 'fn':
                inside = is_entry
            else:
                callee = is_entry
        elif line.startswith('calls=') and callee is not None:
            if callee and not inside:
                allocations += int(line[6:].split()[0])
            callee = None
    return name, instructions, allocations


def callgrind(valgrind, game, resources, work, mods, only):
    """One process under Callgrind; returns {workload: (instructions, allocator calls)} and the lines."""
    prefix = [valgrind, '--tool=callgrind', f'--main-stacksize={STACK}', '--instr-atstart=no', '--dump-instr=no', '--collect-jumps=no',
              '--cache-sim=no', '--branch-sim=no', f'--callgrind-out-file={work}/callgrind.out']
    rows = run_game(game, resources, work, mods, only, prefix)
    counted = {}
    for dump in sorted(work.glob('callgrind.out*')):
        name, instructions, allocations = parse_dump(dump.read_text(errors='replace'))
        if name in rows and instructions is not None:
            counted[name] = (instructions, allocations)
        dump.unlink()
    return counted, rows


def first_line(command):
    try:
        return subprocess.run(command, capture_output=True, text=True).stdout.strip().splitlines()[0]
    except (OSError, IndexError):
        return None


def cpu_model():
    try:
        for line in Path('/proc/cpuinfo').read_text().splitlines():
            if line.startswith('model name'):
                return line.split(':', 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or None


def measure(args):
    game = Path(args.game).resolve()
    resources = Path(args.resources).resolve()
    valgrind = shutil.which(args.valgrind)
    samples = {}  # (workload, metric) -> per-operation values
    checksums, operations, notes = {}, {}, []
    with tempfile.TemporaryDirectory(prefix='hot-paths-', dir=args.work) as work:
        work = Path(work).resolve()
        game = install(game, work)

        def take(rows, mods, sample):
            for name, row in rows.items():
                operations[name] = row['operations']
                seen = checksums.setdefault(name, row['checksum'])
                if seen != row['checksum']:
                    notes.append(f'{name}: sample {mods}/{sample} did different work (checksum differs)')

        for mods in ('none', 'native'):
            for sample in range(args.wall_samples):
                rows = run_game(game, resources, work / f'wall-{mods}-{sample}', mods, args.only)
                take(rows, mods, sample)
                for name, row in rows.items():
                    samples.setdefault((name, 'wall_ns'), []).append(row['wall_ns'] / row['operations'])
            if not valgrind:
                continue
            for sample in range(args.count_samples):
                counted, rows = callgrind(valgrind, game, resources, work / f'count-{mods}-{sample}', mods, args.only)
                take(rows, mods, f'callgrind-{sample}')
                for name, (instructions, allocations) in counted.items():
                    samples.setdefault((name, 'instructions'), []).append(instructions / rows[name]['operations'])
                    samples.setdefault((name, 'allocations'), []).append(allocations / rows[name]['operations'])
    if not valgrind:
        notes.append('instructions and allocations not measured: no valgrind on this machine')
    result = {
        'schema': SCHEMA,
        'commit': args.commit or first_line(['git', 'rev-parse', 'HEAD']),
        'date': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'run': args.run or '',
        'host': args.host,
        'cpu': cpu_model(),
        'kernel': platform.release(),
        'rustc': first_line(['rustc', '--version']),
        'valgrind': first_line([valgrind, '--version']) if valgrind else None,
        'workloads': {name: {'operations': operations[name], 'checksum': checksums[name]} for name in sorted(operations)},
        'metrics': {f'{name}.{metric}': values for (name, metric), values in sorted(samples.items())},
        'notes': notes,
    }
    Path(args.out).write_text(json.dumps(result, indent=1) + '\n')
    print(summary(result))


def clean(result):
    """The result with only the fields a published record may hold, each checked; raises ValueError."""
    def text(key, limit=120, optional=False):
        value = result.get(key)
        if value is None and optional:
            return None
        if not isinstance(value, str) or len(value) > limit or '/' in value or '\\' in value:
            raise ValueError(f'{key}: not a short plain string')
        return value

    if result.get('schema') != SCHEMA:
        raise ValueError(f'schema: expected {SCHEMA}')
    commit = text('commit')
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('commit: not a full hash')
    date = text('date')
    datetime.datetime.strptime(date, '%Y-%m-%dT%H:%M:%SZ')
    run = text('run')
    if not re.fullmatch(r'[0-9]{0,20}(\.[0-9]{1,4})?', run):
        raise ValueError('run: not a run number')
    workloads = {}
    for name, row in result.get('workloads', {}).items():
        if not NAME.match(name) or not isinstance(row.get('operations'), int) \
                or not re.fullmatch(r'[0-9a-f]{16}', str(row.get('checksum'))):
            raise ValueError(f'workload {name}: malformed')
        workloads[name] = {'operations': row['operations'], 'checksum': row['checksum']}
    metrics = {}
    for name, values in result.get('metrics', {}).items():
        workload, _, metric = name.rpartition('.')
        if workload not in workloads or metric not in METRICS or not isinstance(values, list) or not values \
                or len(values) > 32 or not all(isinstance(v, (int, float)) and 0 <= v < 1e15 for v in values):
            raise ValueError(f'metric {name}: malformed')
        metrics[name] = values
    notes = result.get('notes', [])
    if not isinstance(notes, list) or len(notes) > 64:
        raise ValueError('notes: malformed')
    # A note is one of this tool's own sentences about a workload; anything else is dropped.
    notes = [note for note in notes if isinstance(note, str)
             and re.fullmatch(r'[a-z0-9_.]{1,80}: sample [a-z0-9/-]{1,40} did different work \(checksum differs\)'
                              r'|instructions and allocations not measured: no valgrind on this machine', note)]
    return {'schema': SCHEMA, 'commit': commit, 'date': date, 'run': run, 'host': text('host', 64),
            'cpu': text('cpu', optional=True), 'kernel': text('kernel', optional=True),
            'rustc': text('rustc', optional=True), 'valgrind': text('valgrind', optional=True),
            'workloads': workloads, 'metrics': metrics, 'notes': notes}


def record(args):
    result = clean(json.loads(Path(args.result).read_text()))
    runs = Path(args.history) / 'runs'
    runs.mkdir(parents=True, exist_ok=True)
    stamp = result['date'].replace(':', '').replace('-', '')
    name = f"{stamp}-{result['commit'][:12]}{'-' + result['run'] if result['run'] else ''}.json"
    (runs / name).write_text(json.dumps(result, indent=1) + '\n')
    print(runs / name)


def middle(values):
    return statistics.median(values)


def spread(values):
    """How far a metric's samples lie apart, as a share of their mean; None for one sample."""
    if len(values) < 2:
        return None
    mean = statistics.fmean(values)
    return (max(values) - min(values)) / mean if mean else 0.0


def load(history):
    runs = [json.loads(path.read_text()) for path in sorted((Path(history) / 'runs').glob('*.json'))]
    return sorted(runs, key=lambda run: (run['date'], run['run']))


def setting(run, metric):
    """What a run's value of `metric` depends on besides the code: runs that differ in it are not compared."""
    if metric == 'wall_ns':
        return run['host'], run['cpu'], run['rustc']
    return run['cpu'], run['rustc'], run['valgrind']


def series(runs, name):
    """The runs that can be compared for metric `name`, oldest first, as (run, value, spread).

    It is the newest unbroken stretch of runs: a run whose workload did other work (operations or checksum), or
    that was measured on another CPU model or with another rustc or Valgrind (for wall time: another host label,
    CPU model or rustc), ends the stretch before it, so such a change starts a new series and is never a step."""
    workload, _, metric = name.rpartition('.')
    kept = []
    for run in reversed(runs):
        if name not in run['metrics']:
            continue
        if kept and (run['workloads'].get(workload) != kept[0][0]['workloads'][workload]
                     or setting(run, metric) != setting(kept[0][0], metric)):
            break
        values = run['metrics'][name]
        kept.append((run, middle(values), spread(values)))
    return kept[::-1]


def change(points, back):
    """The newest value against the one `back` runs before it, as a share; None when there is none."""
    if len(points) <= back or not points[-1 - back][1]:
        return None
    return points[-1][1] / points[-1 - back][1] - 1


def between_runs(points):
    """The largest difference between runs of the newest run's commit, as a share of their mean."""
    same = [value for run, value, _ in points if run['commit'] == points[-1][0]['commit']]
    return spread(same)


def noise(points):
    """A metric's noise: the larger of the newest run's sample spread and the spread between runs of its commit."""
    found = [n for n in (points[-1][2], between_runs(points)) if n is not None]
    return max(found) if found else None


def stepped(points):
    """Whether the newest value is a step against the run before: over STEP and over three times the noise."""
    moved = change(points, 1)
    if moved is None or points[-1][0]['commit'] == points[-2][0]['commit']:
        return False
    return abs(moved) > max(STEP, 3 * (noise(points) or 0.0))


def number(value):
    if value >= 1000:
        return f'{value:,.0f}'
    return f'{value:.3g}' if value else '0'


def share(value):
    return 'n/a' if value is None else f'{value * 100:+.2f}%'


def chart(points, width=180, height=28):
    """The last 60 values as an inline SVG line, scaled to their own range."""
    values = [value for _, value, _ in points[-60:]]
    if len(values) < 2:
        return ''
    low, high = min(values), max(values)
    scale = (high - low) or 1.0
    step = (width - 4) / (len(values) - 1)
    line = ' '.join(f'{2 + i * step:.1f},{height - 3 - (v - low) / scale * (height - 6):.1f}' for i, v in enumerate(values))
    return (f'<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" '
            f'aria-label="last {len(values)} runs"><polyline fill="none" stroke="#2a6fb0" stroke-width="1.5" '
            f'points="{line}"/></svg>')


def rows(runs):
    """One row per metric of the newest run that has it: what the page and the summary show."""
    names = sorted({name for run in runs for name in run['metrics']})
    out = []
    for name in names:
        points = series(runs, name)
        if not points:
            continue
        found = noise(points)
        out.append({
            'name': name, 'metric': name.rpartition('.')[2], 'last': points[-1][1], 'points': points,
            'changes': [change(points, back) for back in (1, 7, 30)], 'noise': found,
            'noisy': found is not None and found > NOISY, 'step': stepped(points),
        })
    return out


def summary(result):
    """A run's own table, in Markdown, for the job summary."""
    lines = [f"Hot paths at `{result['commit'][:12]}` on `{result['host']}` ({result['cpu']})", '',
             '| Metric | Value per operation | Spread of samples | Samples |', '|---|---:|---:|---:|']
    for name, values in result['metrics'].items():
        found = spread(values)
        mark = ' (noisy)' if found is not None and found > NOISY else ''
        lines.append(f'| `{name}` | {number(middle(values))} | {"n/a" if found is None else f"{found * 100:.3f}%"}{mark} | {len(values)} |')
    lines += ['', *[f'- {note}' for note in result['notes']]]
    return '\n'.join(lines)


def page(args):
    history = Path(args.history)
    runs = load(history)
    if not runs:
        raise SystemExit(f'no runs under {history}/runs')
    newest = runs[-1]
    table = rows(runs)
    head = ('<tr><th>Metric</th><th>Last</th><th>vs 1 run ago</th><th>vs 7</th><th>vs 30</th>'
            '<th>Noise</th><th>Runs</th><th>Trend</th></tr>')
    sections = []
    for metric, title in METRICS.items():
        body = []
        for row in (row for row in table if row['metric'] == metric):
            flags = ''.join(f' <span class="{kind}">{label}</span>' for kind, label, on in
                            (('step', 'step', row['step']), ('noisy', 'noisy', row['noisy'])) if on)
            cells = ''.join(f'<td class="n">{share(value)}</td>' for value in row['changes'])
            found = 'n/a' if row['noise'] is None else f"{row['noise'] * 100:.3f}%"
            body.append(f"<tr><td><code>{html.escape(row['name'].rpartition('.')[0])}</code>{flags}</td>"
                        f"<td class=\"n\">{number(row['last'])}</td>{cells}<td class=\"n\">{found}</td>"
                        f"<td class=\"n\">{len(row['points'])}</td><td>{chart(row['points'])}</td></tr>")
        if body:
            sections.append(f'<h2>{html.escape(title)}</h2>\n<table>{head}\n' + '\n'.join(body) + '\n</table>')
    recent = ''.join(
        f"<tr><td>{html.escape(run['date'])}</td><td><code>{run['commit'][:12]}</code></td>"
        f"<td>{html.escape(run['host'])}</td><td>{html.escape(run['cpu'] or '')}</td>"
        f"<td>{html.escape(run['rustc'] or '')}</td><td>{html.escape(run['valgrind'] or 'none')}</td></tr>"
        for run in reversed(runs[-15:]))
    notes = ''.join(f'<li>{html.escape(note)}</li>' for note in newest['notes'])
    text = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Horizon hot paths</title>
<style>
body {{ font: 14px/1.45 system-ui, sans-serif; margin: 2em auto; max-width: 1100px; padding: 0 1em; color: #1c1c1c; }}
table {{ border-collapse: collapse; width: 100%; margin-bottom: 2em; }}
th, td {{ border-bottom: 1px solid #ddd; padding: 4px 8px; text-align: left; vertical-align: middle; }}
td.n {{ text-align: right; font-variant-numeric: tabular-nums; }}
.step {{ background: #b3261e; color: #fff; padding: 0 5px; border-radius: 3px; font-size: 12px; }}
.noisy {{ background: #e8e2c8; padding: 0 5px; border-radius: 3px; font-size: 12px; }}
</style></head><body>
<h1>Horizon hot paths</h1>
<p>Newest run: {html.escape(newest['date'])}, commit <code>{newest['commit'][:12]}</code>, host {html.escape(newest['host'])}.
{len(runs)} runs recorded. Every value is per operation of a fixed, seeded workload, the median of the run's samples.</p>
<p>Instructions and allocator calls do not depend on the machine's speed. Wall time does, and is noisy.
<b>Noise</b> is the larger of the spread of the newest run's samples and the difference between runs of the same commit.
A metric whose noise is over {NOISY * 100:g}% is marked <span class="noisy">noisy</span>: read no small change from it.
<span class="step">step</span> marks a change against the run before of more than {STEP * 100:g}% and three times the noise.
A series starts again at one run when the workload's work changed (another checksum or operation count) or the
runs were measured on another CPU model or with another rustc or Valgrind. Nothing here fails a build.</p>
{''.join(sections)}
<h2>Newest run's notes</h2><ul>{notes or '<li>none</li>'}</ul>
<h2>Recent runs</h2>
<table><tr><th>Date</th><th>Commit</th><th>Host</th><th>CPU</th><th>Rust</th><th>Valgrind</th></tr>{recent}</table>
</body></html>
"""
    (history / 'index.html').write_text(text)
    stepped_rows = [row['name'] for row in table if row['step']]
    readme = ['# Horizon hot-path history', '',
              'Written by the `hot-paths` workflow on main; do not edit by hand. Open `index.html` for the trend page;',
              '`runs/` holds one JSON record per run.', '',
              f"Newest run: {newest['date']}, commit `{newest['commit'][:12]}`, host `{newest['host']}`.", '',
              'Steps against the run before: ' + (', '.join(f'`{name}`' for name in stepped_rows) or 'none') + '.', '']
    (history / 'README.md').write_text('\n'.join(readme))
    print(f'{history / "index.html"}: {len(table)} metrics over {len(runs)} runs; steps: {len(stepped_rows)}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    m = commands.add_parser('measure')
    m.add_argument('--game', required=True)
    m.add_argument('--resources', required=True)
    m.add_argument('--out', required=True)
    m.add_argument('--work', help='where the scratch folder goes (default: the system temporary folder)')
    m.add_argument('--valgrind', default='valgrind')
    m.add_argument('--wall-samples', type=int, default=5)
    m.add_argument('--count-samples', type=int, default=3)
    m.add_argument('--only', help='workload name prefixes, comma-separated')
    m.add_argument('--commit')
    m.add_argument('--host', default='local', help="a label for the kind of machine, such as CI's runner class")
    m.add_argument('--run', help='the CI run number and attempt, as N.A')
    m.set_defaults(call=measure)
    r = commands.add_parser('record')
    r.add_argument('result')
    r.add_argument('history')
    r.set_defaults(call=record)
    p = commands.add_parser('page')
    p.add_argument('history')
    p.set_defaults(call=page)
    args = parser.parse_args()
    args.call(args)


if __name__ == '__main__':
    sys.exit(main())
