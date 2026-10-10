#!/usr/bin/env python3
"""Compare client-ticked block-entity animation with the vanilla 26.3 block entities.

The Java oracle (VanillaBlockEntityAnimOracle) builds each case's real vanilla
block entity (shulker box, bell, conduit, dragon/piglin head, decorated pot)
on a stand-in client level, then per scripted step applies the block event
(triggerEvent), runs the entity's own client ticker and the renderer's
extractRenderState, printing the fields, the level calls the tick made
(neighbour updates, entity pushes/queries, sounds, particles) and the render
state. `examples/block_entity_anim_fixture` replays the steps through
`horizon_client::block_entity_anim` and derives the render state from the
`BlockAnim` Horizon's renderer reads. Lines must match exactly (floats as bits).

  test_block_entity_anim_parity.py [--filter SUBSTRING] [--detail N]
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
HORIZONTAL = ['north', 'south', 'west', 'east']
ALL = ['down', 'up'] + HORIZONTAL
FRAME = ['minecraft:prismarine', 'minecraft:prismarine_bricks', 'minecraft:sea_lantern', 'minecraft:dark_prismarine']
WATERY = ['minecraft:water', 'minecraft:water[level=3]', 'minecraft:bubble_column', 'minecraft:seagrass',
          'minecraft:kelp_plant', 'minecraft:oak_slab[waterlogged=true]', 'minecraft:oak_stairs[waterlogged=true]']
DRY = ['minecraft:air', 'minecraft:stone', 'minecraft:lava', 'minecraft:oak_slab', 'minecraft:ice']


def frame_offsets():
    out = []
    for ox in range(-2, 3):
        for oy in range(-2, 3):
            for oz in range(-2, 3):
                ax, ay, az = abs(ox), abs(oy), abs(oz)
                if (ax > 1 or ay > 1 or az > 1) and (ox == 0 and (ay == 2 or az == 2) or oy == 0 and (ax == 2 or az == 2)
                                                     or oz == 0 and (ax == 2 or ay == 2)):
                    out.append((ox, oy, oz))
    return out


def cases():
    rng = random.Random(0x414e494d)
    out = []

    def pos():
        return [rng.randint(-3000, 3000), rng.randint(-60, 300), rng.randint(-3000, 3000)]

    def add(kind, block, steps, **extra):
        case = {'name': f'{kind}-{len(out)}', 'block': block, 'pos': pos(), 'steps': steps}
        case.update(extra)
        out.append(case)

    def times(n, start=None, gaps=(1,)):
        t = rng.randint(-100, 10 ** 7) if start is None else start
        result = []
        for _ in range(n):
            result.append(t)
            t += rng.choice(gaps)
        return result

    # Shulker boxes: players open and close (open counts from startOpen/stopOpen), other events ignored.
    for i in range(48):
        color = rng.choice(['', 'white_', 'purple_', 'black_'])
        block = f'minecraft:{color}shulker_box[facing={rng.choice(ALL)}]'
        steps, count = [], 0
        for t in times(rng.randint(20, 70)):
            step = {'time': t, 'partial': round(rng.random(), 4)}
            r = rng.random()
            if r < 0.12:
                count = max(count, 0) + 1
                step['event'] = [1, count]
            elif r < 0.22:
                count -= 1
                step['event'] = [1, count]
            elif r < 0.25:
                step['event'] = [rng.choice([0, 2, 1]), rng.randint(-2, 3)]
            steps.append(step)
        add('shulker', block, steps)

    # Bells: rings from any side (and odd data values), raiders and other mobs nearby.
    for i in range(40):
        attach = rng.choice(['floor', 'ceiling', 'single_wall', 'double_wall'])
        block = f'minecraft:bell[attachment={attach},facing={rng.choice(HORIZONTAL)}]'
        p = pos()
        entities = []
        for _ in range(rng.choice([0, 1, 2, 5, 12, 25])):
            # Multiples of 1/1024: serde_json parses other decimals approximately (1 ulp off).
            spread = rng.choice([10, 33, 50, 70])
            entities.append({'pos': [p[k] + 0.5 + round(rng.uniform(-spread, spread) * 1024) / 1024 for k in range(3)],
                             'raider': rng.random() < 0.5, 'alive': rng.random() < 0.85, 'removed': rng.random() < 0.08})
        steps = []
        start = rng.randint(-50, 10 ** 6)
        for k, t in enumerate(times(rng.randint(60, 180), start)):
            step = {'time': t, 'partial': round(rng.random(), 4)}
            if (k == 2 or rng.random() < 0.05) and k < 150:
                step['event'] = [1 if rng.random() < 0.93 else 2, rng.choice([0, 1, 2, 3, 4, 5, 2, 3, 4, 5, 7, -3, 11])]
            steps.append(step)
        case = {'name': f'bell-{len(out)}', 'block': block, 'pos': p, 'steps': steps, 'entities': entities}
        out.append(case)

    # Conduits: water around, prismarine frames of various sizes, frames changing over time.
    rings = frame_offsets()
    for i in range(36):
        world = {}
        water = rng.random()
        for ox in range(-1, 2):
            for oy in range(-1, 2):
                for oz in range(-1, 2):
                    if (ox, oy, oz) != (0, 0, 0):
                        world[f'{ox},{oy},{oz}'] = rng.choice(WATERY) if rng.random() < 0.97 or water < 0.5 else rng.choice(DRY)
        size = rng.choice([0, 10, 15, 16, 20, 30, 41, 42, 42])
        for o in rng.sample(rings, size):
            world['%d,%d,%d' % o] = rng.choice(FRAME)
        steps = []
        for t in times(rng.randint(60, 140), gaps=(1, 1, 1, 2, 7)):
            step = {'time': t, 'partial': round(rng.random(), 4)}
            if rng.random() < 0.04:
                o = rng.choice(rings)
                step['set'] = {'%d,%d,%d' % o: rng.choice(FRAME + DRY)}
            steps.append(step)
        waterlogged = 'true' if rng.random() < 0.9 else 'false'
        extra = {'world': world, 'seed': rng.randint(0, 2 ** 40)}
        if rng.random() < 0.3:
            extra['target'] = {'pos': [round(rng.uniform(-8, 8) * 1024) / 1024 for _ in range(3)], 'width': round(rng.uniform(0.3, 2), 3),
                               'height': round(rng.uniform(0.5, 3), 3), 'eye': round(rng.uniform(0.2, 2.5), 3)}
        add('conduit', f'minecraft:conduit[waterlogged={waterlogged}]', steps, **extra)

    # Dragon and piglin heads powered on and off.
    for i in range(24):
        head = rng.choice(['dragon', 'piglin'])
        wall = rng.random() < 0.5
        prop = f'facing={rng.choice(HORIZONTAL)}' if wall else f'rotation={rng.randint(0, 15)}'
        name = f'minecraft:{head}_wall_head' if wall else f'minecraft:{head}_head'
        powered = rng.random() < 0.5
        steps = []
        for t in times(rng.randint(20, 90)):
            if rng.random() < 0.1:
                powered = not powered
            steps.append({'time': t, 'partial': round(rng.random(), 4),
                          'state': f'{name}[{prop},powered={str(powered).lower()}]'})
        add('skull', f'{name}[{prop},powered=false]', steps)

    # Decorated pots wobbling (insert: POSITIVE, fail: NEGATIVE) and invalid events.
    for i in range(24):
        steps = []
        for t in times(rng.randint(20, 60), gaps=(0, 1, 1, 1, 3)):
            step = {'time': t, 'partial': round(rng.random(), 4)}
            if rng.random() < 0.15:
                step['event'] = [rng.choice([1, 1, 1, 2]), rng.choice([0, 1, 0, 1, 2, -1])]
            steps.append(step)
        add('pot', f'minecraft:decorated_pot[facing={rng.choice(HORIZONTAL)}]', steps)
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-block-entity-anim-parity'
    build.mkdir(parents=True, exist_ok=True)
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    chosen = [c for c in cases() if args.filter in c['name']]
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'VanillaBlockEntityAnimOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaBlockEntityAnimOracle',
         inputs, build / 'vanilla.txt'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client', '--example', 'block_entity_anim_fixture', '--',
         resources, inputs, build / 'rust.txt'], 'rust')

    def split(path):
        result, current = {}, None
        for line in path.read_text().splitlines():
            if line.startswith('case '):
                current = line[5:]
                result[current] = []
            else:
                result[current].append(line)
        return result
    vanilla, rust = split(build / 'vanilla.txt'), split(build / 'rust.txt')
    kinds, failures, errors, steps = {}, {}, {}, 0
    for case in chosen:
        name = case['name']
        kind = name.split('-')[0]
        v, r = vanilla.get(name, []), rust.get(name, [])
        total = kinds.setdefault(kind, [0, 0])
        total[1] += 1
        if v and v[0].startswith('error'):
            errors.setdefault(kind, v[0])
            continue
        steps += len(v)
        bad = next((i for i, (a, b) in enumerate(zip(v, r)) if a != b), None)
        if bad is None and len(v) != len(r):
            bad = min(len(v), len(r))
        if bad is None:
            total[0] += 1
        else:
            failures.setdefault(kind, []).append((case, bad, v[bad] if bad < len(v) else None, r[bad] if bad < len(r) else None))
    for kind, error in sorted(errors.items()):
        print(f'ORACLE ERROR {kind}: {error[:600]}')
    for kind, items in sorted(failures.items()):
        print(f'MISMATCH {kind}: {len(items)} cases')
        for case, step, a, b in items[:args.detail]:
            print(f'    {case["name"]} {case["block"]} step {step}: {json.dumps(case["steps"][step] if step < len(case["steps"]) else None)}')
            print(f'      vanilla: {(a or "")[:500]}')
            print(f'      horizon: {(b or "")[:500]}')
    for kind, (ok, n) in sorted(kinds.items()):
        print(f'{kind}: {ok}/{n} cases match')
    passed = sum(ok for ok, _ in kinds.values())
    print(f'{passed}/{len(chosen)} cases match ({steps} ticked steps)')
    if failures or errors:
        sys.exit(1)
    print('block entity animation parity ok')


if __name__ == '__main__':
    main()
