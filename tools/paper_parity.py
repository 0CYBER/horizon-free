#!/usr/bin/env python3
"""Paper event / Horizon hook traces. Build the native trace engine before `run`.

All generated data lives in .local/paper-parity. No CI or ledger updates.

Radius finding (view/simulation distance 2, chunk-load-igxvxjrc): Paper loads
radius 5; Horizon 4. Keep this mismatch, including FULL/border chunks.
Vanilla 26.3 server-game.jar (javap -c -p): ChunkMap.setServerViewDistance
passes 2 unchanged to DistanceManager.updatePlayerTickets; PlayerTicketTracker
places level-31 PLAYER_LOADING tickets at distance <= 2. ChunkLevel.fullStatus
includes level 33 as FULL, hence radius 4. Horizon lan_update_streaming uses
view + 1, and streaming::Ranges finishes one more ring: also radius 4.
Paper documents FULL as loaded border chunks: https://docs.papermc.io/paper/reference/commands/#chunkinfo
Its global chunk-loading settings control sending/rates/concurrency, not a
vanilla-radius switch; unload delay cannot grow the union of loaded chunks.
See https://docs.papermc.io/paper/reference/global-configuration/#chunk-loading-advanced
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time
from types import SimpleNamespace
import zipfile
import zlib

from lra_schem import _nbt

import lab
import oracle_harness as oracle
import server_bench as bench

FIXTURES = lab.ROOT / 'tools/fixtures'
SCENARIOS = lab.ROOT / 'tools/paper_parity_scenarios.json'
HOME_KEYS = ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME',
             'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD', 'HORIZON_LAUNCHER_DESKTOP')


def hook_map():
    """Read catalog attributes, including multiline attributes and Paper event lists."""
    mapping = {}
    for path in (lab.ROOT / 'crates/horizon-mod-vocab/src/hooks').glob('*.rs'):
        for attrs, hook in re.findall(r'#\[hook\((.*?)\)\]\s*fn\s+(\w+)\s*\(', path.read_text(), re.S):
            if match := re.search(r'\bpaper\s*=\s*("[^"]+"|\[[^\]]*\])', attrs):
                mapping[hook] = [s.strip() for v in re.findall(r'"([^"]+)"', match[1])
                                 for s in v.split(',')]
    return mapping


def chunk_fields(side, fields):
    # Per-hook normalization; event names still come exclusively from the catalog.
    if side == 'paper':
        fields = dict(fields, dimension=fields['world'])
    else:
        fields = dict(fields, x=fields['chunk'][0], z=fields['chunk'][1])
    value = tuple(fields[k] for k in ('dimension', 'x', 'z', 'new'))
    if not (isinstance(value[0], str) and type(value[1]) is int
            and type(value[2]) is int and type(value[3]) is bool):
        raise ValueError('invalid chunk fields')
    return value


def hotbar_fields(side, fields):
    value = tuple(fields[k] for k in (('old', 'new') if side == 'paper' else ('old_slot', 'new_slot')))
    if any(type(v) is not int or not 0 <= v <= 8 for v in value):
        raise ValueError('invalid hotbar fields')
    return value


def moisture_fields(side, fields):
    pos, before, after = tuple(fields['pos']), fields['from'], fields['to']
    if len(pos) != 3 or any(type(v) is not int for v in pos + (before, after)) or not (0 <= before <= 7 and 0 <= after <= 7):
        raise ValueError('invalid moisture fields')
    return pos, before, after


NORMALIZE = {'chunk_load': chunk_fields, 'player_switch_hotbar_slot_pre': hotbar_fields,
             'moisture_change': moisture_fields}


def observations(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    joins = [i for i, r in enumerate(rows) if r['name'] == '_join']
    quits = [i for i, r in enumerate(rows) if r['name'] == '_quit']
    if len(joins) != 1 or len(quits) != 1 or quits[0] <= joins[0]:
        raise ValueError(f'{path.name}: expected one ordered join/quit, got join rows {joins}, quit rows {quits}')
    return rows[joins[0] + 1:quits[0]]


def sequence(side, label, got, expected):
    for i in range(max(len(got), len(expected))):
        actual = got[i] if i < len(got) else '<missing>'
        wanted = expected[i] if i < len(expected) else '<end>'
        if actual != wanted:
            raise ValueError(f'{side}: {label} row {i + 1}: expected {wanted!r}, got {actual!r}')


def final_blocks(out, side, positions):
    # Read the stopped server's saved overworld, including 26.3's compact palette entries.
    raw = (out / side / 'world/dimensions/minecraft/overworld/region/r.0.0.mca').read_bytes()
    offset = int.from_bytes(raw[:3], 'big') * 4096
    size = int.from_bytes(raw[offset:offset + 4], 'big')
    if not offset or raw[offset + 4] != 2:
        raise ValueError(f'{side}: final blocks: expected zlib chunk (0,0), got missing/unsupported chunk')
    chunk = _nbt(zlib.decompress(raw[offset + 5:offset + 4 + size]))
    result = []
    for x, y, z in positions:
        if not (0 <= x < 16 and 0 <= z < 16):
            raise ValueError(f'{side}: final block position outside fixture chunk: {(x, y, z)}')
        blocks = next(s['block_states'] for s in chunk['sections'] if s['Y'] == y // 16)
        palette = blocks['palette']
        bits = max(4, (len(palette) - 1).bit_length())
        i, per = (y % 16) * 256 + z * 16 + x, 64 // bits
        entry = palette[(blocks['data'][i // per] >> (i % per * bits)) & ((1 << bits) - 1)
                        if 'data' in blocks else 0]
        name = entry.get('id', entry.get('Name', entry.get('')))
        props = entry.get('properties', entry.get('Properties', {}))
        result.append(name + (f"[moisture={props['moisture']}]" if 'moisture' in props else ''))
    return result


def compare_contract(out, manifest, hook, events):
    scenario, cancel = manifest['scenario'], manifest.get('cancel', {})
    sides = [observations(out / f'{side}.jsonl') for side in ('paper', 'horizon')]
    values = []
    for side, rows, names in zip(('paper', 'horizon'), sides, (events, [hook])):
        selected = [r for r in rows if r['name'] in names]
        if hook == 'moisture_change':
            selected = [r for r in selected if r['fields']['pos'] in scenario['positions']]
        if not selected:
            raise ValueError(f'{side}: {hook} row 1: expected an event, got <missing>')
        for i, r in enumerate(selected):
            if type(r['tick']) is not int or type(r['cancelled']) is not bool:
                raise ValueError(f'{side}: {hook} row {i + 1}: expected integer tick and boolean cancellation, got {r}')
        values.append([(NORMALIZE[hook](side, r['fields']), r['cancelled'], r['tick']) for r in selected])
    if hook == 'player_switch_hotbar_slot_pre':
        expected, slots, slot = [], [], 0
        for tick, new in scenario['bot']['script']:
            if new != slot:
                denied = new == cancel.get(hook, {}).get('new')
                expected.append(((slot, new), denied, tick))
                slot = slot if denied else new
                slots.append(slot)
        for side, rows, values_side in zip(('paper', 'horizon'), sides, values):
            sequence(side, 'hotbar (old,new,cancelled)', [(v, c) for v, c, _ in values_side],
                     [(v, c) for v, c, _ in expected])
            sequence(side, 'selected slot', [r['fields']['slot'] for r in rows if r['name'] == '_slot'], slots)
        relative = [[t - vs[0][2] for _, _, t in vs] for vs in [*values, expected]]
        for i, ts in enumerate(zip(*relative)):
            if max(ts) - min(ts) > 1:
                raise ValueError(f'hotbar tick tolerance row {i + 1}: expected script +{ts[2]} +/-1; '
                                 f'got paper +{ts[0]}, horizon +{ts[1]} (spread <=1 required)')
        posts = [(hotbar_fields('horizon', r['fields']), r['tick']) for r in sides[1]
                 if r['name'] == 'player_switch_hotbar_slot_post']
        sequence('horizon', 'post', posts, [(v, t) for v, c, t in values[1] if not c])
    else:
        expected = set()
        for pos in scenario['positions']:
            denied = pos == cancel.get(hook, {}).get('pos')
            transitions = [(7, 6)] if denied else ([(0, 7)] if pos[2] == 10 else [(n, n-1) for n in range(7, 0, -1)])
            expected.update(((tuple(pos), a, b), denied) for a, b in transitions)
        for side, rows, vs in zip(('paper', 'horizon'), sides, values):
            sequence(side, 'moisture transitions (sorted set)', sorted({(v, c) for v, c, _ in vs}), sorted(expected))
            windows = [[r['tick'] for r in rows if r['name'] == name]
                       for name in ('_window_start', '_window_end')]
            if any(len(w) != 1 for w in windows) or windows[1][0] - windows[0][0] != scenario['window_ticks']:
                raise ValueError(f'{side}: random-tick window: expected one start/end {scenario["window_ticks"]} ticks apart, got {windows}')
            for pos, got in zip(scenario['positions'], final_blocks(out, side, scenario['positions'])):
                expected_blocks = (['minecraft:farmland[moisture=7]'] if pos[2] == 10 or pos == cancel.get(hook, {}).get('pos')
                                   else ['minecraft:dirt', 'minecraft:grass_block'])
                if got not in expected_blocks:
                    raise ValueError(f'{side}: final block {pos}: expected {expected_blocks}, got {got!r}')
    return scenario['tolerance']


def trace(path, names, side, normalize):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    joins = [i for i, row in enumerate(rows) if row['name'] == '_join']
    quits = [i for i, row in enumerate(rows) if row['name'] == '_quit']
    if len(joins) != 1 or len(quits) != 1 or quits[0] <= joins[0]:
        raise ValueError(f'{path.name}: expected one ordered join/quit, got join rows {joins}, quit rows {quits}')
    result = set()
    for row in rows[:quits[0]]:
        if row['name'] in names:
            if type(row['tick']) is not int or row['cancelled'] is not False:
                raise ValueError(f'{side}: expected integer tick and cancelled=false, got {row}')
            result.add(normalize(side, row['fields']))
    return result


def report(out, covered_only=False):
    manifest = json.loads((out / 'run.json').read_text())
    if 'runs' in manifest:
        return int(any([report(out / path, True) for path in manifest['runs']]))
    mapping, failed = hook_map(), False
    for hook, events in sorted(mapping.items()):
        if hook not in manifest['scenario']['hooks'] or hook not in NORMALIZE:
            if not covered_only:
                print(f'{hook}: not covered')
            continue
        try:
            if manifest.get('error') or not manifest.get('complete'):
                raise ValueError(manifest.get('error', 'incomplete run'))
            if hook != 'chunk_load':
                detail = compare_contract(out, manifest, hook, events)
                print(f"{out.name}/{hook}: match ({detail})")
                continue
            left = trace(out / 'paper.jsonl', events, 'paper', NORMALIZE[hook])
            right = trace(out / 'horizon.jsonl', [hook], 'horizon', NORMALIZE[hook])
            if not left or not right:
                raise ValueError(f'chunk_load: expected nonempty sets; got paper={len(left)}, horizon={len(right)}')
            if left != right:
                raise ValueError(f'Paper-only {len(left - right)} {sorted(left - right)[:3]}; '
                                 f'Horizon-only {len(right - left)} {sorted(right - left)[:3]}')
            print(f'{hook}: match ({len(left)} chunks; set, tick/order ignored)')
        except (OSError, ValueError, KeyError, TypeError, IndexError, StopIteration, zlib.error) as error:
            failed = True
            print(f'{out.name}/{hook}: mismatch: {error}')
    if any(h not in mapping for h in manifest['scenario']['hooks']):
        print('mismatch: scenario hook missing from catalog')
        failed = True
    return int(failed)


def environment(directory):
    env = os.environ.copy()
    for key in HOME_KEYS:
        env[key] = tempfile.mkdtemp(prefix=key.lower() + '-', dir=directory)
    return lab.game_env(env)


def properties(directory, scenario, port):
    directory.mkdir()
    props = dict(bench.PROPERTIES, **{'server-ip': '127.0.0.1', 'server-port': port,
        'level-seed': scenario['seed'], 'level-name': 'world', 'level-type': 'minecraft\\:flat',
        'view-distance': 2, 'simulation-distance': 2, 'generate-structures': 'false',
        'allow-flight': 'true', 'pause-when-empty-seconds': -1})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')


def compile_plugin(out, jar):
    # Paperclip is a bootstrap jar: its first start populates the cached API/dependency jars.
    shared = lab.LOCAL / 'server-bench/paper-shared'
    dependencies = os.pathsep.join(map(str, [jar, *sorted(shared.rglob('*.jar'))]))
    classes = out / 'classes'
    classes.mkdir()
    oracle.run_logged(['javac', '--release', str(lab.config()[0]['minecraft']['java_major']),
        '-proc:none', '-cp', dependencies, '-d', classes,
        FIXTURES / 'paper-trace-plugin/PaperTrace.java'], out, 'compile-java-PaperTrace')
    plugin = out / 'paper-trace.jar'
    with zipfile.ZipFile(plugin, 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.write(FIXTURES / 'paper-trace-plugin/plugin.yml', 'plugin.yml')
        for path in classes.rglob('*.class'):
            archive.write(path, path.relative_to(classes).as_posix())
    return plugin


def run_case(args, scenario, cancel, out):
    out.mkdir()
    manifest = {'schema': 1, 'scenario': scenario, 'complete': False,
                'bot_uuid': str(lab.offline_uuid('Bot0')), 'cancel': cancel}
    lab.write_json(out / 'run.json', manifest)
    try:
        installed = args.engine.parent / 'horizon-native-mods/paper_trace/src/lib.rs'
        if installed.read_bytes() != (FIXTURES / 'paper-trace-mod/src/lib.rs').read_bytes():
            raise ValueError('export does not contain the current paper_trace fixture; rebuild it')
        # Reuse PaperMC's checksum-verified downloader and the pinned Mojang cache setup.
        boot = out / 'bootstrap'
        properties(boot, scenario, lab.free_port())
        paper = bench.server_command('paper', boot, SimpleNamespace(heap=args.heap))
        jar = Path(paper[paper.index('-jar') + 1])
        manifest['paper_jar_sha256'] = lab.digest(jar, 'sha256')
        manifest['engine_sha256'] = lab.digest(args.engine, 'sha256')
        server = bench.Server(paper, boot, out / 'bootstrap.log', environment(boot))
        try:
            server.wait_for(bench.DONE, args.startup_timeout)
        finally:
            server.stop()
        plugin = compile_plugin(out, jar)
        spawns = []
        for side in ('paper', 'horizon'):
            directory, port = out / side, lab.free_port()
            properties(directory, scenario, port)
            env = environment(directory)
            trace_path = out / f'{side}.jsonl'
            config_path = directory / 'trace-config.json'
            config = dict(cancel)
            if 'window_ticks' in scenario:
                config['window_ticks'] = scenario['window_ticks']
            lab.write_json(config_path, config)
            if side == 'paper':
                (directory / 'bukkit.yml').write_text('settings:\n  connection-throttle: -1\n')
                # Spigot's speed check can pull the bot back as it rises from spawn (timing-dependent).
                (directory / 'spigot.yml').write_text(
                    'settings:\n  moved-too-quickly-multiplier: 1000.0\n  moved-wrongly-threshold: 1000.0\n')
                (directory / 'plugins').mkdir()
                shutil.copyfile(plugin, directory / 'plugins/paper-trace.jar')
                command = [paper[0], f'-Dpaper.trace={trace_path}', f'-Dpaper.trace.config={config_path}', *paper[1:]]
            else:
                env['HORIZON_PAPER_TRACE'] = str(trace_path)
                env['HORIZON_PAPER_TRACE_CONFIG'] = str(config_path)
                command = [args.engine, '--resources', args.resources, '--dedicated', '--nogui']
            server = bench.Server(command, directory, out / f'{side}.log', env)
            try:
                server.wait_for(bench.DONE, args.startup_timeout)
                for key, value in scenario['gamerules'].items():
                    server.console(f'gamerule {key} {str(value).lower()}')
                # Ground-level spawn avoids Horizon's unloaded-column Y fallback (initial_spawn.rs:346).
                # player_movement_check=false allows moves batched by slow ticks; still reject corrections.
                for command in scenario['setup']:
                    server.console(command)
                server.console('say PAPER_PARITY_SETUP_DONE')
                server.wait_for('PAPER_PARITY_SETUP_DONE', 600)
                bot = scenario['bot']
                summary = out / f'{side}-bots.json'
                oracle.run_logged([args.bots, '--server', f'127.0.0.1:{port}', '--bots', '1',
                    '--behaviour', bot['behaviour'], '--seed', str(scenario['seed']),
                    '--view-distance', '2', '--height', str(bot['height']),
                    '--explore-speed', str(bot['speed']), '--explore-distance', str(bot['distance']),
                    '--seconds', str(scenario['duration']), '--report', summary], out, f'{side}-bots',
                    timeout=scenario['duration'] + 180, env=env)
                result = json.loads(summary.read_text())
                if (result['joined'] != 1 or result['pulled_back'] or
                        any(b['error'] or (bot['behaviour'] in ('explore', 'hotbar') and not b['explore_complete'])
                            for b in result['per_bot'])):
                    raise ValueError(f'{side}: bot failed, corrected, or did not finish path')
                spawn = [json.loads(l)['spawn'] for l in (out / f'{side}-bots.log').read_text().splitlines()
                         if l.startswith('{"spawn":')]
                if len(spawn) != 1:
                    raise ValueError(f'{side}: missing spawn position')
                spawns.append(spawn[0])
                deadline = time.monotonic() + 600
                while not trace_path.exists() or '"_quit"' not in trace_path.read_text():
                    if time.monotonic() >= deadline:
                        raise ValueError(f'{side}: no quit marker (trace mod/plugin missing?)')
                    time.sleep(0.1)
            finally:
                server.stop()
        if spawns[0] != spawns[1]:
            raise ValueError(f'different path origins: {spawns}')
        manifest['complete'] = True
    except Exception as error:
        manifest['error'] = str(error)
    lab.write_json(out / 'run.json', manifest)
    print(f'Traces and logs: {out}')
    return report(out, True)


def run(args):
    scenarios = json.loads(SCENARIOS.read_text())
    out = Path(tempfile.mkdtemp(prefix='run-', dir=lab.LOCAL / 'paper-parity'))
    runs = []
    for name in scenarios if args.scenario == 'all' else [args.scenario]:
        scenario = scenarios[name]
        for label, cancel in [('allow', {}), *([('cancel', scenario['cancel'])] if 'cancel' in scenario else [])]:
            path = f'{name}-{label}'
            runs.append(path)
            run_case(args, scenario, cancel, out / path)
    lab.write_json(out / 'run.json', {'runs': runs})
    print(f'Combined report: {out}')
    return report(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    start = commands.add_parser('run')
    start.add_argument('--engine', type=Path, required=True, help='export containing paper_trace')
    start.add_argument('--resources', type=Path, default=lab.RESOURCES)
    start.add_argument('--bots', type=Path, default=lab.cargo_target() / f'fast/horizon-bots{bench.EXE}')
    start.add_argument('--heap', default='2g')
    start.add_argument('--startup-timeout', type=int, default=1500,
                       help="seconds to wait for a server's Done line (resource loading is slow on network drives)")
    start.add_argument('--scenario', choices=['all', *json.loads(SCENARIOS.read_text())], default='all')
    diff = commands.add_parser('report')
    diff.add_argument('out', type=Path)
    args = parser.parse_args()
    if args.command == 'report':
        return report(args.out.resolve())
    for key in ('engine', 'resources', 'bots'):
        setattr(args, key, getattr(args, key).resolve())
    cache = lab.LOCAL / 'paper-parity'
    cache.mkdir(parents=True, exist_ok=True)
    # tempfile, Java and tool subprocesses must also stay beside the worktree.
    scratch = cache / 'tmp'
    scratch.mkdir(exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ.update(TEMP=str(scratch), TMP=str(scratch), TMPDIR=str(scratch))
    return run(args)


if __name__ == '__main__':
    sys.exit(main())
