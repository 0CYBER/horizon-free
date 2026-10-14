#!/usr/bin/env python3
"""Compare arrows (AbstractArrow.tick) over the native client's resident scene.

Real vanilla `Arrow` entities tick in an independently built copy of the
scene (`VanillaArrowOracle`, client and server jars); Horizon's
`arrows::tick` runs over the scene itself (`examples/arrow_probe.rs`). Every
operation prints the arrow's position, motion, rotation, ground state, life,
shake, stuck block, fire ticks, water flags, fluid heights, removal, crit flag
and fall distance by exact bits, then its sounds, particles and game events in
order. Cases: flights in the air with and without crits or physics, into and
through water (inertia, bubbles, splashes, currents of flowing water, eyes),
kelp, seagrass and waterlogged blocks, bubble columns with and without drag
and with air or water above, sticking in full and partial blocks, falling out
when the block goes, starting inside a block, and despawning after 1200 ticks.
No entities, owner, rain, lava or fire.
"""
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_native_client as client
except ImportError:
    import lab
    import test_rust_parity as base
    import test_native_client as client


def bits(value): return struct.pack('>d', value).hex()
def vec(values): return list(map(bits, values))
def op(mode, value=None, **extra): return dict(mode=mode, value=value, **extra)


def corpus():
    rng = random.Random(0x4152524f57)
    def place(name, positions, properties=None):
        return [op('block', name='minecraft:' + name, position=list(p), **({'properties': properties} if properties else {})) for p in positions]
    def restore(positions):
        return [op('block', name='minecraft:air', position=list(p)) for p in positions]
    def arrow(position, velocity, ops, crit=False, no_physics=False):
        return dict(seed=str(rng.randrange(-1 << 63, 1 << 63)), position=vec(position), velocity=vec(velocity),
                    crit=crit, no_physics=no_physics, ops=[op('level_seed', str(rng.randrange(1 << 48)))] + ops)
    ticks = lambda n: [op('tick') for _ in range(n)]
    def aim(speed, yaw, pitch):
        import math
        y, p = math.radians(yaw), math.radians(pitch)
        return (-math.sin(y) * math.cos(p) * speed, -math.sin(p) * speed, math.cos(y) * math.cos(p) * speed)
    # Flights in the air onto the floor (y 4), sticking, then resting.
    for _ in range(40):
        start = (rng.uniform(17, 30), rng.uniform(5, 14), rng.uniform(2, 30))
        v = aim(rng.choice([.3, 1, 1.6, 2.4, 3]), rng.uniform(-180, 180), rng.uniform(-60, 45))
        yield arrow(start, v, ticks(60), crit=rng.random() < .5)
    # Without physics: straight through blocks, no gravity, inverted yaw.
    for _ in range(6):
        start = (rng.uniform(17, 30), rng.uniform(5, 8), rng.uniform(2, 30))
        yield arrow(start, aim(1.5, rng.uniform(-180, 180), rng.uniform(-30, 30)), ticks(25), no_physics=True)
    # Water: a pool three deep, shot into from above, through it and from inside.
    pool = [(x, y, z) for x in range(20, 29) for y in range(4, 7) for z in range(20, 29)]
    for level in ['0', '0', '0', '2', '6', '8']:
        for _ in range(8):
            start = rng.choice([(rng.uniform(20, 29), rng.uniform(8, 14), rng.uniform(16, 19)),
                                (rng.uniform(21, 28), rng.uniform(4.2, 6.5), rng.uniform(21, 28))])
            v = aim(rng.choice([.05, .4, 1.2, 2.5, 3]), rng.uniform(-60, 60), rng.uniform(-50, 20))
            ops = place('water', pool, {'level': level}) + ticks(70) + restore(pool)
            yield arrow(start, v, ops, crit=rng.random() < .3)
    # Currents: random flowing water around a slow arrow.
    for _ in range(16):
        cells = [(x, 4, z) for x in range(21, 28) for z in range(21, 28)]
        ops = [op('block', name='minecraft:water', position=list(c), properties={'level': rng.choice(['0', '1', '3', '5', '7', '9'])}) for c in cells]
        ops += ticks(40) + restore(cells)
        yield arrow((rng.uniform(21.3, 27.7), rng.uniform(4.05, 4.6), rng.uniform(21.3, 27.7)), aim(rng.choice([0, .01, .2]), rng.uniform(-180, 180), 0), ops)
    # Kelp, seagrass and waterlogged blocks are water too.
    for name, properties in [('kelp', {'age': '5'}), ('kelp_plant', {}), ('seagrass', {}), ('oak_slab', {'type': 'bottom', 'waterlogged': 'true'}),
                             ('oak_fence', {'waterlogged': 'true'}), ('iron_bars', {'waterlogged': 'true'})]:
        blocks = [(x, y, z) for x in range(22, 27) for y in range(4, 6) for z in range(22, 27)]
        for _ in range(4):
            v = aim(rng.choice([.6, 1.5, 2.8]), rng.uniform(-40, 40), rng.uniform(5, 60))
            ops = place(name, blocks, properties) + ticks(50) + restore(blocks)
            yield arrow((rng.uniform(22, 27), rng.uniform(7, 10), rng.uniform(18, 20)), v, ops)
    # Bubble columns: up or down, open above or under water.
    for drag in ['true', 'false']:
        for roof in [None, 'water', 'stone']:
            column = [(24, y, 24) for y in range(4, 9)]
            ops = place('bubble_column', column, {'drag': drag})
            if roof:
                ops += place(roof, [(24, 9, 24)])
            for start, v in [((24.5, 12, 24.5), (0, -.3, 0)), ((24.5, 4.2, 24.5), (0, .4, 0)), ((24.2, 6, 24.7), (.02, 0, -.01)), ((22, 9, 24.5), (.5, -.2, 0))]:
                yield arrow(start, v, ops + ticks(60) + restore(column + [(24, 9, 24)]))
    # Sticking in partial blocks, then the block goes (or changes) under it.
    shapes = [('stone', {}), ('oak_slab', {'type': 'top'}), ('oak_slab', {'type': 'bottom'}), ('oak_stairs', {'facing': 'east'}),
              ('oak_fence', {}), ('glass_pane', {'north': 'true', 'south': 'true'}), ('oak_trapdoor', {'half': 'top'}),
              ('lantern', {}), ('iron_chain', {'axis': 'y'}), ('white_carpet', {}), ('snow', {'layers': '3'}), ('anvil', {'facing': 'north'})]
    for name, properties in shapes:
        for after in ['air', 'glass', None]:
            target = [(25, 9, 25)]
            v = rng.choice([(0, -1.2, 0), (.9, .1, 0), (0, .1, -.9), (.7, -.7, .2)])
            start = (25.5 - v[0] * 3, 9.5 - v[1] * 3, 25.5 - v[2] * 3)
            ops = place(name, target, properties) + ticks(20)
            if after:
                ops += place(after, target)
            ops += ticks(40) + restore(target)
            yield arrow(start, v, ops, crit=True)
    # Starting inside a block: stopped in place.
    for name in ['stone', 'oak_slab', 'oak_fence']:
        yield arrow((25.5, 9.25, 25.5), (.4, 0, .1), place(name, [(25, 9, 25)]) + ticks(10) + restore([(25, 9, 25)]))
    # Despawning 1200 ticks after sticking in the floor.
    for _ in range(2):
        yield arrow((rng.uniform(18, 30), 6, rng.uniform(4, 30)), (0, -1, 0), ticks(1215))


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-arrow-parity'
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    resource_hash = client.resource_fingerprint(resources)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaArrowOracle.java'
    helpers = [oracle.with_name(name + '.java') for name in ['VanillaSceneCollisionOracle', 'VanillaRegistryCollisionScanOracle',
        'VanillaHeightmapOracle', 'VanillaCollisionOracle', 'VanillaBlockStateCodecOracle', 'VanillaSuffocationOracle',
        'VanillaFaceSupportOracle', 'VanillaEntityAttributeOracle', 'VanillaBlockLandingOracle', 'VanillaMovementRegistryOracle', 'VanillaDamageSourceOracle']]
    sources = [Path(__file__), lab.ROOT / 'tools/test_native_client.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               oracle, *helpers, lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/horizon-client/examples/arrow_probe.rs']
    for crate in ['horizon-core', 'horizon-world', 'horizon-assets', 'horizon-client']:
        sources += [lab.ROOT / 'crates' / crate / 'Cargo.toml', *sorted((lab.ROOT / 'crates' / crate / 'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars.get('server', jars['client'])) + os.pathsep + dependencies,
         '-d', classes, oracle, *helpers], 'compile-java')
    fixtures = list(corpus())
    operations = sum(len(f['ops']) for f in fixtures)
    expected = 2 * operations + len(fixtures)
    (build / 'inputs.txt').write_text(''.join(json.dumps(f, separators=(',', ':')) + '\n' for f in fixtures))
    input_hash = lab.digest(build / 'inputs.txt', 'sha256')
    run(['cargo', 'build', '--locked', '-p', 'horizon-client', '--example', 'arrow_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('arrow_probe.exe' if os.name == 'nt' else 'arrow_probe')
    run([binary, resources, build, '--describe'], 'describe')
    references = []
    for side, jar in jars.items():
        run(['java', '-Xmx768M', '-cp', os.pathsep.join([str(classes), str(jar), dependencies]),
             'dev.horizonlab.parity.VanillaArrowOracle', side, build, resources], side)
        run([binary, resources, build, side], 'rust-' + side)
        matched = base.compare(build / (side + '.txt'), build / ('rust-' + side + '.txt'))
        if matched != expected:
            raise AssertionError('Incomplete arrow output')
        references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'), 'output_sha256': lab.digest(build / (side + '.txt'), 'sha256'),
                           'matched_output_lines': matched, 'rust_output_sha256': lab.digest(build / ('rust-' + side + '.txt'), 'sha256')})
        print(f'{side}: {matched:,} arrow states match', flush=True)
    base.ensure_sources_unchanged(sources, tested)
    if resource_hash != client.resource_fingerprint(resources) or input_hash != lab.digest(build / 'inputs.txt', 'sha256'):
        raise AssertionError('Arrow inputs changed during verification')
    lab.write_json(report, {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
        'platform': platform.platform(), 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0], 'fixtures': len(fixtures),
        'operations': operations, 'sources_sha256': tested, 'resources': resource_hash, 'input_sha256': input_hash,
        'references': references, 'scope': __doc__})
    print(f'Arrow parity passed: {report}', flush=True)


if __name__ == '__main__':
    main()
