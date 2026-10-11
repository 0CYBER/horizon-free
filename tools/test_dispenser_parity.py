#!/usr/bin/env python3
"""Compare vanilla dispensers and droppers with Horizon's, one activation per case.

A dispenser or dropper at the origin faces a random direction and holds the
case's items; the block in front (and below it) comes from a palette of blocks
the behaviours care about, and some cases put a target entity in front. Vanilla
`DispenserBlock.dispenseFrom` runs over the headless level of the block update
oracle (`VanillaDispenserOracle`); Horizon runs its game glue in a headless game
(`HORIZON_DISPENSE_FIXTURE`). Both print block changes, container contents,
spawned entities (type, position, motion), the targets afterwards, level events,
sounds, game events and the level random. Motion drawn from an entity's own
unseeded random (vanilla marks it "~") is compared within its spread.
"""
import json
import os
import random
import subprocess
try:
    from . import block_update_harness as h, lab, test_rust_parity as base
except ImportError:
    import block_update_harness as h
    import lab
    import test_rust_parity as base

DIRECTIONS = ['down', 'up', 'north', 'south', 'west', 'east']
STEP = {'down': (0, -1, 0), 'up': (0, 1, 0), 'north': (0, 0, -1), 'south': (0, 0, 1), 'west': (-1, 0, 0), 'east': (1, 0, 0)}


def potion(item, name):
    return '{id:"minecraft:%s",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:%s"}}}' % (item, name)


def plain(item, count=1):
    return '{id:"minecraft:%s",count:%d}' % (item, count)


# Items with the blocks and entities in front that exercise their behaviour.
ITEMS = {
    'drop': [plain('stone', 5), plain('diamond'), plain('stick', 64), plain('oak_sapling', 3), plain('redstone', 2)],
    'projectile': [plain('arrow', 3), potion('tipped_arrow', 'poison'), plain('spectral_arrow'), plain('egg', 2), plain('blue_egg'),
                   plain('brown_egg'), plain('snowball', 4), plain('experience_bottle'), potion('splash_potion', 'healing'),
                   potion('lingering_potion', 'swiftness'), plain('firework_rocket', 2), plain('fire_charge', 2), plain('wind_charge', 3)],
    'bucket': [plain('water_bucket'), plain('lava_bucket'), plain('powder_snow_bucket'), plain('salmon_bucket'), plain('cod_bucket'),
               plain('pufferfish_bucket'), plain('tropical_fish_bucket'), plain('axolotl_bucket'), plain('tadpole_bucket'),
               plain('sulfur_cube_bucket'), plain('bucket'), plain('bucket', 3)],
    'block': [plain('bone_meal', 4), plain('tnt', 2), plain('flint_and_steel'), plain('glowstone', 3), plain('honeycomb', 2),
              plain('glass_bottle', 2), potion('potion', 'water'), potion('potion', 'healing'), plain('shulker_box'),
              plain('red_shulker_box'), plain('carved_pumpkin'), plain('wither_skeleton_skull'), plain('shears'), plain('brush')],
    'vehicle': [plain('oak_boat'), plain('bamboo_raft'), plain('minecart'), plain('chest_minecart'),
                plain('furnace_minecart'), plain('tnt_minecart'), plain('hopper_minecart'), plain('armor_stand', 2)],
    'spawn': [plain('pig_spawn_egg'), plain('sheep_spawn_egg', 2), plain('zombie_spawn_egg'), plain('cow_spawn_egg'), plain('chicken_spawn_egg')],
    'equipment': [plain('iron_helmet'), plain('diamond_chestplate'), plain('leather_leggings'), plain('golden_boots'),
                  plain('turtle_helmet'), plain('saddle'), plain('iron_horse_armor'), plain('white_carpet'), plain('chest'),
                  plain('carved_pumpkin'), plain('elytra'), plain('wolf_armor'), plain('shears'), plain('brush'), plain('player_head')],
}

FRONT = {
    'drop': ['air', 'stone', 'water', 'chest', 'hopper'],
    'projectile': ['air', 'air', 'water', 'stone', 'glass'],
    'bucket': ['air', 'air', 'water', 'water', 'lava', 'powder_snow', 'oak_slab', 'seagrass', 'stone', 'cauldron', 'kelp',
               'grass_block', 'fire'],
    'block': ['air', 'grass_block', 'wheat', 'oak_sapling', 'tnt', 'campfire', 'candle', 'respawn_anchor', 'beehive', 'bee_nest',
              'copper_block', 'exposed_cut_copper_stairs', 'dirt', 'coarse_dirt', 'rooted_dirt', 'water', 'seagrass', 'stone',
              'sweet_berry_bush', 'cocoa', 'obsidian', 'soul_campfire', 'candle_cake', 'netherrack', 'moss_block'],
    'vehicle': ['air', 'air', 'water', 'rail', 'powered_rail', 'activator_rail', 'stone', 'detector_rail'],
    'spawn': ['air', 'air', 'water', 'stone', 'grass_block'],
    'equipment': ['air'],
}
BELOW = ['air', 'stone', 'water', 'rail', 'grass_block', 'soul_sand', 'dirt', 'farmland', 'netherrack']

TARGETS = {
    'iron_helmet': ['zombie', 'skeleton', 'armor_stand', 'pig'],
    'diamond_chestplate': ['zombie', 'armor_stand'],
    'leather_leggings': ['skeleton', 'armor_stand'],
    'golden_boots': ['zombie', 'armor_stand'],
    'turtle_helmet': ['armor_stand', 'zombie'],
    'saddle': ['pig', 'horse', 'strider', 'donkey', 'camel', 'llama'],
    'iron_horse_armor': ['horse', 'donkey'],
    'white_carpet': ['llama', 'horse', 'trader_llama'],
    'chest': ['donkey', 'mule', 'llama', 'horse'],
    'carved_pumpkin': ['zombie', 'armor_stand', 'snow_golem'],
    'elytra': ['armor_stand', 'zombie'],
    'wolf_armor': ['wolf'],
    'shears': ['sheep', 'sheep', 'mooshroom', 'bogged', 'snow_golem'],
    'brush': ['armadillo'],
    'tnt': ['sulfur_cube'],
    'flint_and_steel': ['sulfur_cube'],
    'stone': ['sulfur_cube'],
    'player_head': ['armor_stand', 'zombie'],
}


class States:
    def __init__(self, facts):
        self.facts = facts

    def get(self, name, **props):
        name = name if ':' in name else 'minecraft:' + name
        best = self.facts.defaults[name]
        if not props:
            return best
        for index in self.facts.by_block[name]:
            have = self.facts.states[index][4].get('properties', {})
            if all(have.get(k) == v for k, v in props.items()):
                return index
        raise KeyError((name, props))

    def any(self, rng, name):
        name = name if ':' in name else 'minecraft:' + name
        return rng.choice(self.facts.by_block[name])


def front_state(states, rng, name):
    """Mostly the default state, sometimes a random one (charges, honey, ages, lit...)."""
    if name in ('respawn_anchor', 'beehive', 'bee_nest', 'campfire', 'candle', 'wheat', 'sweet_berry_bush', 'rail', 'powered_rail',
                'candle_cake', 'soul_campfire') or rng.random() < 0.3:
        return states.any(rng, name)
    return states.get(name)


def cases(facts, count, seed, only):
    rng = random.Random(seed)
    states = States(facts)
    kinds = list(ITEMS)
    for n in range(count):
        kind = kinds[n % len(kinds)]
        item = rng.choice(ITEMS[kind])
        if only and only not in item:
            continue
        facing = rng.choice(DIRECTIONS)
        block = 'dropper' if rng.random() < 0.12 else 'dispenser'
        cells = [[0, 0, 0, states.get(block, facing=facing, triggered='false')]]
        dx, dy, dz = STEP[facing]
        front = (dx, dy, dz)
        below = (dx, dy - 1, dz)
        front_name = rng.choice(FRONT[kind])
        if front_name != 'air':
            h.put(cells, front, front_state(states, rng, front_name))
        if below != (0, 0, 0) and rng.random() < 0.8:
            name = rng.choice(BELOW)
            if name != 'air':
                h.put(cells, below, front_state(states, rng, name))
        # A few random blocks around (portal frames, fire supports, neighbours).
        for _ in range(rng.randrange(0, 6)):
            at = (rng.randrange(-2, 3), rng.randrange(-2, 3), rng.randrange(-2, 3))
            if at in ((0, 0, 0), front):
                continue
            h.put(cells, at, states.get(rng.choice(['stone', 'obsidian', 'dirt', 'water', 'soul_sand', 'grass_block', 'oak_planks'])))
        items = []
        slots = rng.sample(range(9), rng.choice([1, 1, 1, 2, 3, 9]))
        for i, slot in enumerate(slots):
            items.append([0, 0, 0, slot, item if i == 0 or rng.random() < 0.6 else plain('cobblestone', rng.randrange(1, 65))])
        case = {**h.environment(rng), 'mode': 'dispense', 'pos': [0, 0, 0], 'cells': cells, 'items': items}
        case.pop('raining', None)
        case.pop('thundering', None)
        if rng.random() < 0.08:
            case['rules'] = {'tnt_explodes': False}
        bare = item.split('"')[1].split(':')[1]
        if kind == 'equipment' or (bare in TARGETS and rng.random() < 0.5):
            entities = []
            for _ in range(rng.choice([1, 1, 1, 2])):
                target = {'type': 'minecraft:' + rng.choice(TARGETS.get(bare, ['zombie'])),
                          'pos': [dx + 0.5, dy + rng.choice([0.0, 0.0, 0.25]), dz + 0.5], 'yaw': rng.choice([0.0, 90.0, 180.0])}
                for flag, odds in (('baby', 0.2), ('tame', 0.6), ('sheared', 0.25), ('chest', 0.15), ('loot', 0.5), ('brown', 0.3)):
                    if rng.random() < odds:
                        target[flag] = True
                if target['type'] == 'minecraft:sulfur_cube' and rng.random() < 0.5:
                    target['equip'] = {'body': plain(rng.choice(['tnt', 'stone', 'slime_block']))}
                entities.append(target)
            case['entities'] = entities
        if block == 'dropper' and rng.random() < 0.5:
            h.put(cells, front, states.get(rng.choice(['chest', 'hopper', 'barrel', 'dispenser'])))
            if rng.random() < 0.5:
                items.append([dx, dy, dz, 0, plain(rng.choice(['stone', 'diamond', 'stick']), rng.randrange(1, 65))])
        yield case


# Items whose behaviour runs Mob.finalizeSpawn (spawn eggs, mob buckets). The
# finalization draws belong to the entity spawning port; for these cases the
# level random's end state is not compared, everything else is.
FINALIZED = ('_spawn_egg', 'salmon_bucket', 'cod_bucket', 'pufferfish_bucket', 'tropical_fish_bucket', 'axolotl_bucket',
             'tadpole_bucket', 'sulfur_cube_bucket')


def finalizes(case):
    return any(any(tag in i[4] for tag in FINALIZED) for i in case['items'] if i[:3] == [0, 0, 0])

def run(command, **kwargs):
    done = subprocess.run(command, capture_output=True, text=True, **kwargs)
    if done.returncode:
        print((done.stdout + done.stderr)[-4000:])
        raise SystemExit(f'{command[0]} failed')
    return done


def numbers_close(java, rust):
    """Token-wise equality; a "~" token is an unseeded entity draw and only needs to fall in its spread."""
    a, b = java.split(' '), rust.split(' ')
    if len(a) != len(b):
        return False
    for x, y in zip(a, b):
        if x.startswith('~'):
            try:
                if abs(float(x[1:]) - float(y)) > 0.25:
                    return False
            except ValueError:
                return False
        elif x != y:
            return False
    return True


def same(java, rust):
    java = sorted(java)
    rust = sorted(rust)
    if len(java) != len(rust):
        return False
    pending = list(rust)
    for line in java:
        if line in pending:
            pending.remove(line)
            continue
        if '~' not in line:
            return False
        match = next((r for r in pending if numbers_close(line, r)), None)
        if match is None:
            return False
        pending.remove(match)
    return True


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 3000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'dispenser-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, args.count, args.seed, args.only))
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in all_cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
         '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaDispenserOracle.java')])
    run(['java', '-Xmx3G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaDispenserOracle',
         str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    run(['cargo', 'build', '-q', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
        env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    run([str(lab.cargo_target() / 'release/horizon-client'), '--resources', str(resources)], cwd=work,
        env={**os.environ, 'HORIZON_DISPENSE_FIXTURE': f'{work / "cases.jsonl"},{work / "rust.txt"}', 'HORIZON_NO_AUDIO': '1'})
    # Loot table names (D) are vanilla's bookkeeping; the drops themselves are compared.
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix) + ('D ',)
    native = h.split_cases((work / 'native.txt').read_text(), ignore)
    rust = h.split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(all_cases), (len(native), len(rust), len(all_cases))
    groups, errors = {}, {}
    unseeded = 0
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        if finalizes(case):
            unseeded += 1
            # The spawned mob's variant (and so its voice) is chosen in finalizeSpawn too.
            a = [line for line in a if not line.startswith('rng ') and not line.endswith('.ambient')]
            b = [line for line in b if not line.startswith('rng ') and not line.endswith('.ambient')]
        if not same(a, b):
            item = next(i[4] for i in case['items'] if i[:3] == [0, 0, 0])
            block = facts.states[case['cells'][0][3]][0].split(':')[1]
            groups.setdefault(f'{block} {item.split(chr(34))[1]}', []).append((case, a, b))
    for kind, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {kind}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + describe(case, facts))
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors; '
          f'{unseeded} spawning cases compared without the final level random)')
    (work / 'differences.json').write_text(json.dumps({k: [[c, a, b] for c, a, b in v[:5]] for k, v in groups.items()}))
    h.finish(total)


def describe(case, facts):
    cells = ' '.join(f'{c[0]},{c[1]},{c[2]}={facts.describe(c[3])}' for c in case['cells'])
    extra = ''
    if case.get('entities'):
        extra += ' entities=' + json.dumps(case['entities'])
    if case.get('rules'):
        extra += ' rules=' + json.dumps(case['rules'])
    return f'{cells} items={json.dumps(case["items"])}{extra} seed={case["seed"]}'


if __name__ == '__main__':
    main()
