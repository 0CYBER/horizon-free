#!/usr/bin/env python3
"""Compare the in-game HUD decorations with the vanilla 26.3 client's Hud.

The Java oracle (VanillaHudOracle) runs vanilla's own Hud.extractCrosshair,
extractItemHotbar, extractPlayerHealth, extractVehicleHealth, ExperienceBar
and ContextualBar.extractExperienceLevel on a stand-in client whose player
answers from the case, and records every blit (sprite, position, size,
partial region), text call (string, position, colour, shadow) and hotbar
item (slot, position, seed, pose). `examples/hud_fixture` lays out the same
state with `horizon_client::hud_layout`; the ordered call lists must agree.

  test_hud_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
EFFECTS = ['minecraft:poison', 'minecraft:wither', 'minecraft:regeneration', 'minecraft:hunger', 'minecraft:speed']


def cases(count):
    rng = random.Random(0x485544)
    out = []
    for n in range(count):
        max_health = rng.choice([20.0, 20.0, 6.0, 40.0, 33.0, 100.0])
        health = rng.choice([max_health, round(rng.uniform(0, max_health), 2), 1.0, 0.5, 3.0])
        air_max = 300
        under = rng.random() < 0.3
        air = rng.choice([air_max, rng.randint(-20, air_max), 0, 1, 29, 30, 31, 299]) if under or rng.random() < 0.2 else air_max
        case = {
            'width': rng.choice([427, 320, 480, 640, 961]), 'height': rng.choice([240, 255, 270, 360, 501]),
            'gameMode': rng.choice(['survival'] * 5 + ['adventure', 'creative', 'spectator']),
            'crosshair': rng.random() < 0.7,
            'attackIndicator': rng.choice(['OFF', 'CROSSHAIR', 'HOTBAR']),
            'attackScale': rng.choice([1.0, round(rng.random(), 3), 0.0, 0.999]),
            'selected': rng.randint(0, 8), 'rightHanded': rng.random() < 0.7,
            'slots': [rng.choice([None, 0, 0, 3, 5]) if rng.random() < 0.6 else None for _ in range(9)],
            'offhand': rng.choice([None, None, 0, 4]), 'partial': round(rng.random(), 3),
            'health': health, 'maxHealth': max_health,
            'absorption': rng.choice([0.0, 0.0, 4.0, 3.5, 17.0, 40.0]),
            'armor': rng.choice([0, 0, 1, 7, 20, 13]), 'food': rng.randint(0, 20),
            'saturation': rng.choice([0.0, 5.0, 0.0]),
            'effects': [e for e in EFFECTS if rng.random() < 0.15], 'frozen': rng.random() < 0.08,
            'hardcore': rng.random() < 0.2, 'damageCooldown': rng.choice([0, 0, 10]),
            'air': air, 'maxAir': air_max, 'underWater': under,
            'tick': rng.randint(0, 100000), 'lastHealth': rng.randint(0, 40), 'displayHealth': rng.randint(0, 40),
            'lastHealthAgo': rng.choice([0, 500, 5000]), 'healthBlinkTime': 0,
            'xpProgress': rng.choice([0.0, round(rng.random(), 4), 0.999]), 'xpLevel': rng.choice([0, 1, 7, 30, 123, 4000]),
            'experienceBar': rng.random() < 0.8,
        }
        if case['damageCooldown'] and rng.random() < 0.5:
            case['healthBlinkTime'] = case['tick'] + rng.randint(1, 20)
        if rng.random() < 0.15:
            case['vehicleMaxHealth'] = rng.choice([15.0, 30.0, 53.0, 100.0])
            case['vehicleHealth'] = round(rng.uniform(0, case['vehicleMaxHealth']), 1)
        out.append(case)
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=600)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-hud-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    chosen = cases(args.count)
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaHudOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaHudOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client', '--example', 'hud_fixture', '--',
         resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust)
    errors, bad = [], []
    for v, r in zip(vanilla, rust):
        if 'error' in v:
            errors.append(v['error']); continue
        a = [{k: x for k, x in d.items() if k != 'color' or d['type'] == 'text'} for d in v['draws']]
        for d in a:
            if d['type'] == 'sprite':
                d.setdefault('region', None)
        b = r['draws']
        def same(x, y):
            if x.keys() != y.keys():
                return False
            return all(
                (all(abs(p - q) <= 1e-4 * (1 + abs(p)) for p, q in zip(x[k], y[k])) if k == 'matrix' else x[k] == y[k]) for k in x)
        if len(a) != len(b) or not all(same(x, y) for x, y in zip(a, b)):
            first = next((i for i, (x, y) in enumerate(zip(a, b)) if not same(x, y)), min(len(a), len(b)))
            bad.append((v['case'], first, a[first:first + 2], b[first:first + 2], len(a), len(b)))
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for case, i, a, b, la, lb in bad[:args.detail]:
        print('MISMATCH', json.dumps(case))
        print(f'   at call {i} ({la} vanilla, {lb} Horizon calls)\n   vanilla {a}\n   horizon {b}')
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} HUD states match; {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('hud parity ok')


if __name__ == '__main__':
    main()
