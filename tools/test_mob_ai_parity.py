#!/usr/bin/env python3
"""Differential mob AI harness.

The Java oracle (VanillaMobAiOracle) runs vanilla mobs in a real, headless, flat vanilla server
level: each scenario builds a scene, places a target player and mobs with seeded random sources
and ticks the mobs the way ServerLevel.tick does. The Rust probe (mob_ai_probe) replays the same
scenarios on Horizon's mob AI. Both print every mob's AI-visible state after every tick (position,
motion, rotations, running goals and target goals, attack target, navigation path, move and look
controls, movement input, brain activities and memories) and the sounds played; this harness
reports, per scenario and mob, how many ticks match before the first divergence and what differs.

  python3 tools/test_mob_ai_parity.py [--only NAME ...] [--rust-only] [--java-only] [--report-only] [--trace]

The oracle traces every mob random draw with its call stack; --trace builds the probe with
the rng-trace feature and line tables so the report shows Horizon's draws next to vanilla's.
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import oracle_harness
    import test_rust_parity as base
    import test_model_baking_parity as baking

FLOOR = -60  # top of the flat preset's grass layer + 1


def ring(x0, z0, size, state, y=FLOOR):
    return [f'fill {x0 - size} {y} {z0 - size} {x0 + size} {y} {z0 + size} {state}']


# Each scenario is (name, ticks, options, lines); positions are relative to the scenario origin
# and translated by the generator so scenarios never share chunks.
def scenarios():
    s = []

    def add(name, ticks, lines, opts=''):
        s.append((name, ticks, opts, lines))

    # Passive and hostile land mobs on open grass with a survival player nearby.
    for species, seed in [('zombie', 1), ('skeleton', 2), ('creeper', 3), ('spider', 4), ('husk', 5),
                          ('cow', 6), ('pig', 7), ('chicken', 8), ('sheep', 9), ('wandering_trader', 10),
                          ('villager', 11), ('drowned', 12), ('enderman', 13), ('witch', 14)]:
        add(f'{species}_player', 200, [
            'player 6.5 -60 0.5',
            f'mob minecraft:{species} 0.5 -60 0.5 0 {seed} persistent',
        ])
        add(f'{species}_alone', 200, [
            f'mob minecraft:{species} 0.5 -60 0.5 90 {seed * 7 + 1} persistent',
        ])
    # Drowned in a pool with the player on the shore and in the water.
    add('drowned_pool', 200, ['fill -6 -64 -6 6 -61 6 minecraft:water',
                              'player 9.5 -60 0.5',
                              'mob minecraft:drowned 0.5 -63 0.5 0 21 persistent'])
    add('drowned_pool_swimmer', 200, ['fill -6 -64 -6 6 -61 6 minecraft:water',
                                      'player 3.5 -62 0.5',
                                      'mob minecraft:drowned -3.5 -63 0.5 0 22 persistent'])
    # Night: undead do not burn, monsters seek darkness.
    add('zombie_night', 200, ['player 10.5 -60 0.5', 'mob minecraft:zombie 0.5 -60 0.5 0 31 persistent'], 'time=18000')
    add('skeleton_day_burn', 120, ['mob minecraft:skeleton 0.5 -60 0.5 0 32 persistent'])
    # The wither (summoned, not built: no invulnerable spawn phase).
    add('wither_player', 200, ['player 10.5 -60 0.5', 'mob minecraft:wither 0.5 -58 0.5 0 41 persistent'])
    # A villager with a job site, a bed and a bell.
    add('villager_village', 300, [
        'block 4 -60 0 minecraft:composter',
        'block -4 -60 2 minecraft:red_bed[facing=north,part=foot]',
        'block -4 -60 1 minecraft:red_bed[facing=north,part=head]',
        'block 0 -60 -6 minecraft:bell',
        'player 12.5 -60 0.5',
        'mob minecraft:villager 0.5 -60 0.5 0 51 persistent',
    ])
    # Breezes: idle, and fighting a player near, in, and beyond the inner jump circle.
    add('breeze_alone', 200, ['mob minecraft:breeze 0.5 -60 0.5 90 61 persistent'])
    add('breeze_player', 200, ['player 8.5 -60 0.5', 'mob minecraft:breeze 0.5 -60 0.5 0 62 persistent'])
    add('breeze_close', 200, ['player 2.5 -60 1.5', 'mob minecraft:breeze 0.5 -60 0.5 0 63 persistent'])
    add('breeze_far', 200, ['player 13.5 -60 6.5', 'mob minecraft:breeze 0.5 -60 0.5 0 64 persistent'])
    # Creakings: the player faces +z, so it watches one and turns its back on the other.
    add('creaking_alone', 200, ['mob minecraft:creaking 0.5 -60 0.5 90 71 persistent'])
    add('creaking_watched', 200, ['player 0.5 -60 -7.5', 'mob minecraft:creaking 0.5 -60 0.5 180 72 persistent'])
    add('creaking_behind', 200, ['player 0.5 -60 8.5', 'mob minecraft:creaking 0.5 -60 0.5 0 73 persistent'])
    # Woken while watched, then free to hunt and hit once the player turns away.
    add('creaking_attack', 240, ['player 0.5 -60 -7.5 turn=40,180', 'mob minecraft:creaking 0.5 -60 0.5 180 74 persistent'])
    # A creaking heart in a pale oak trunk: by night it spawns its creaking near a player,
    # by day it sleeps, without logs it is uprooted, with no player near it waits.
    TRUNK = ['block 3 -60 3 minecraft:pale_oak_log[axis=y]', 'block 3 -58 3 minecraft:pale_oak_log[axis=y]']
    add('heart_night', 300, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'time=18000 spawning')
    add('heart_day', 120, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'spawning')
    add('heart_uprooted', 60, ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'time=18000 spawning')
    # A player hits the heart's creaking: no damage, but the heart bleeds resin and trails.
    add('heart_hurt', 260, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5',
                            'hurt 60 0', 'hurt 64 0', 'hurt 200 0'], 'time=18000 spawning')
    add('heart_far', 120, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 40.5 -60 0.5'], 'time=18000 spawning')
    return s


def write_scenarios(path, chosen):
    lines = []
    for i, (name, ticks, opts, body) in chosen:
        ox, oz = 512 * (i + 1), 0
        lines.append(f'scenario {name} ticks={ticks} {opts}'.rstrip())
        lines.append(f'origin {ox} {oz}')
        for line in body:
            f = line.split()
            if f[0] == 'player':
                f[1] = repr(float(f[1]) + ox)
                f[3] = repr(float(f[3]) + oz)
            elif f[0] == 'mob':
                f[2] = repr(float(f[2]) + ox)
                f[4] = repr(float(f[4]) + oz)
            elif f[0] in ('block', 'heart'):
                f[1] = str(int(f[1]) + ox)
                f[3] = str(int(f[3]) + oz)
            elif f[0] == 'fill':
                for k in (1, 4):
                    f[k] = str(int(f[k]) + ox)
                for k in (3, 6):
                    f[k] = str(int(f[k]) + oz)
            lines.append(' '.join(f))
        lines.append('end')
    path.write_text('\n'.join(lines) + '\n')


def parse(path):
    """{scenario: {tick: {mob: {field: value}}, sounds: {tick: [..]}}}"""
    result, cur = {}, None
    for line in path.read_text().splitlines():
        if line.startswith('== '):
            cur = result.setdefault(line[3:], {'ticks': {}, 'sounds': {}})
            continue
        head, *fields = line.split('\t')
        parts = head.split(' ', 2)
        t = int(parts[0])
        if parts[1] == 'sound':
            cur['sounds'].setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'trace':
            cur.setdefault('trace', {}).setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'heart':
            cur.setdefault('hearts', {}).setdefault(t, []).append(line.split(' ', 2)[2])
            continue
        mob = int(parts[1])
        row = {}
        if len(parts) > 2:
            row['removed'] = parts[2]
        for f in fields:
            k, _, v = f.partition(' ')
            row[k] = v
        cur['ticks'].setdefault(t, {})[mob] = row
    return result


def decode(value):
    """Hex float/double fields made readable."""
    def one(x):
        try:
            if len(x) > 8:
                return f'{struct.unpack(">d", int(x, 16).to_bytes(8, "big"))[0]!r}'
            return f'{struct.unpack(">f", int(x, 16).to_bytes(4, "big"))[0]:.9g}'
        except ValueError:
            return x
    out = []
    for word in value.split(' '):
        out.append(','.join(one(p) if all(c in '0123456789abcdef' for c in p) and len(p) >= 5 else p for p in word.split(',')))
    return ' '.join(out)


MULT, MASK = 0x5DEECE66D, (1 << 48) - 1


def draws(before, after, limit=20000):
    """How many LegacyRandomSource steps lead from one state to the next."""
    try:
        s, goal = int(before, 16), int(after, 16)
    except (TypeError, ValueError):
        return '?'
    for n in range(limit):
        if s == goal:
            return n
        s = (s * MULT + 11) & MASK
    return f'>{limit}'


def compare(vanilla, rust, fields=None):
    """Returns (report lines, totals)."""
    report, totals = [], {'mob_ticks': 0, 'matched': 0, 'scenarios': 0, 'clean': 0}
    for name, v in vanilla.items():
        r = rust.get(name)
        totals['scenarios'] += 1
        if r is None:
            report.append(f'{name}: missing from Horizon')
            continue
        mobs = sorted({m for t in v['ticks'].values() for m in t})
        clean = True
        for mob in mobs:
            ticks = sorted(v['ticks'])
            first = None
            for t in ticks:
                a = v['ticks'][t].get(mob)
                b = r['ticks'].get(t, {}).get(mob)
                totals['mob_ticks'] += 1
                if a == b or (b is not None and fields and all(a.get(k) == b.get(k) for k in fields)):
                    if first is None:
                        totals['matched'] += 1
                    continue
                if first is None:
                    first = t
                    diffs = []
                    for k in (a or {}):
                        if b is None or a.get(k) != b.get(k):
                            show = (lambda x: x) if k == 'rng' else decode
                            diffs.append(f'{k}: vanilla {show(a.get(k, "?"))} | horizon {show((b or {}).get(k, "?"))}')
                    for k in (b or {}):
                        if a is None or k not in a:
                            diffs.append(f'{k}: vanilla ? | horizon {decode(b[k])}')
                    prev_v = v['ticks'].get(t - 1, {}).get(mob, {}).get('rng')
                    prev_r = r['ticks'].get(t - 1, {}).get(mob, {}).get('rng')
                    report.append(f'{name} mob {mob}: first divergence at tick {t} of {len(ticks)} '
                                  f'(random draws this tick: vanilla {draws(prev_v, (a or {}).get("rng"))}, '
                                  f'horizon {draws(prev_r, (b or {}).get("rng"))})')
                    for line in [x for x in v.get('trace', {}).get(t, []) if x.split(' ', 1)[0] == str(mob)][:40]:
                        report.append('      vanilla ' + line)
                    # Present when the probe is built with the rng-trace feature (see --trace).
                    for line in [x for x in r.get('trace', {}).get(t, []) if x.split(' ', 1)[0] == str(mob)][:40]:
                        report.append('      horizon ' + line)
                    report.extend('    ' + d for d in diffs[:12])
            if first is not None:
                clean = False
        # Creaking hearts (state, block entity, level random): first differing tick.
        for t in sorted(set(v.get('hearts', {})) | set(r.get('hearts', {}))):
            if v.get('hearts', {}).get(t) != r.get('hearts', {}).get(t):
                report.append(f'{name}: hearts differ at tick {t}: vanilla {v.get("hearts", {}).get(t)} | horizon {r.get("hearts", {}).get(t)}')
                clean = False
                break
        for t in sorted(set(v['sounds']) | set(r['sounds'])):
            if v['sounds'].get(t, []) != r['sounds'].get(t, []):
                report.append(f'{name}: sounds differ at tick {t}: vanilla {v["sounds"].get(t, [])} | horizon {r["sounds"].get(t, [])}')
                clean = False
                break
        totals['clean'] += clean
        if clean:
            report.append(f'{name}: all {len(v["ticks"])} ticks match')
    return report, totals


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--only', nargs='*')
    parser.add_argument('--java-only', action='store_true')
    parser.add_argument('--rust-only', action='store_true', help='reuse the last vanilla output')
    parser.add_argument('--report-only', action='store_true', help='print divergences without failing')
    parser.add_argument('--trace', action='store_true',
                        help="also trace Horizon's mob random draws (rng-trace probe with line tables)")
    args = parser.parse_args()
    build = lab.LOCAL / 'mob-ai-parity'
    build.mkdir(parents=True, exist_ok=True)
    chosen = [(i, s) for i, s in enumerate(scenarios()) if not args.only or any(o in s[0] for o in args.only)]
    inputs = build / 'scenarios.txt'
    write_scenarios(inputs, chosen)
    lock, jars, _ = base.verify_artifacts()
    if not args.rust_only:
        classes, dependencies, _ = oracle_harness.compile_oracle(build, 'VanillaMobAiOracle', jars, baking.client_libraries(),
                                                                lock['minecraft']['java_major'])
        # The oracle's server world: cleared before each run, never left in /tmp.
        world = build / 'world'
        shutil.rmtree(world, ignore_errors=True)
        oracle_harness.run_logged(['java', '-Xmx3g', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
                                   'dev.horizonlab.parity.VanillaMobAiOracle', inputs, build / 'vanilla.txt', world],
                                  build, 'vanilla', timeout=3600)
    if not args.java_only:
        command = ['cargo', 'build', '--locked', '--release', '-p', 'horizon-client', '--example', 'mob_ai_probe']
        target, env = lab.cargo_target(), None
        if args.trace:
            target = lab.LOCAL / 'mob-ai-trace-target'
            command += ['--features', 'rng-trace']
            env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_PROFILE_RELEASE_DEBUG='line-tables-only',
                       CARGO_PROFILE_RELEASE_LTO='false', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16')
        oracle_harness.run_logged(command, build, 'compile-rust', env=env)
        binary = target / 'release/examples/mob_ai_probe'
        oracle_harness.run_logged([binary, lab.ROOT / '.local/horizon-client/26.3', inputs, build / 'horizon.txt'], build, 'horizon')
    if args.java_only:
        return
    names = {s[0] for _, s in chosen}
    vanilla = {k: v for k, v in parse(build / 'vanilla.txt').items() if k in names}
    report, totals = compare(vanilla, parse(build / 'horizon.txt'))
    (build / 'report.txt').write_text('\n'.join(report) + '\n')
    print('\n'.join(report))
    print(f"{totals['clean']}/{totals['scenarios']} scenarios match entirely; "
          f"{totals['matched']}/{totals['mob_ticks']} mob-ticks match before each mob's first divergence")
    if not args.report_only and totals['clean'] != totals['scenarios']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
