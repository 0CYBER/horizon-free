#!/usr/bin/env python3
"""Compare cauldron item interactions with vanilla's CauldronInteractions.

The Java oracle (VanillaCauldronOracle) calls the cauldron block's useItemOn
with a recording level and player; the Rust example (cauldron_fixture) runs
horizon_client::cauldron. Every item is used on every cauldron state (empty,
water, lava and powder snow at each level), with water potions, dyed armour,
patterned banners and shulker boxes, in survival and creative, under water or
not, and with a crowded inventory. The result, block writes, sounds, drops and
the final inventory are compared.
"""
import argparse
import json
import random
try:
    from . import interaction_lab as lib, lab
except ImportError:
    import interaction_lab as lib
    import lab

STATES = ['minecraft:cauldron|', 'minecraft:water_cauldron|level=1', 'minecraft:water_cauldron|level=2',
          'minecraft:water_cauldron|level=3', 'minecraft:lava_cauldron|', 'minecraft:powder_snow_cauldron|level=1',
          'minecraft:powder_snow_cauldron|level=3']
DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple',
        'blue', 'brown', 'green', 'red', 'black']


def snbt(item, count=1, **components):
    body = ','.join('"minecraft:%s":%s' % (k, v) for k, v in sorted(components.items()))
    return '{id:"%s",count:%d%s}' % (item, count, ',components:{%s}' % body if body else '')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=7)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('cauldron-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaCauldronOracle')
    items = json.loads((lib.RESOURCES / 'items.json').read_text())
    patterns = sorted('minecraft:' + p.stem for p in (lib.RESOURCES / 'data/minecraft/banner_pattern').glob('*.json'))
    rng = random.Random(args.seed)
    held = []
    for name, info in items.items():
        if name == 'minecraft:air':
            continue
        top = info.get('minecraft:max_stack_size', 64)
        held.append(snbt(name, 1))
        held.append(snbt(name, rng.randrange(1, top + 1)))
    for color in DYES:
        for layers in range(0, 4):
            body = '[%s]' % ','.join('{pattern:"%s",color:"%s"}' % (rng.choice(patterns), rng.choice(DYES)) for _ in range(layers))
            held.append(snbt('minecraft:%s_banner' % color, rng.choice([1, 3]), banner_patterns=body))
        held.append(snbt('minecraft:%s_shulker_box' % color, rng.choice([1, 2])))
    for item in ['leather_helmet', 'leather_chestplate', 'leather_leggings', 'leather_boots', 'leather_horse_armor', 'wolf_armor']:
        held.append(snbt('minecraft:' + item, 1, dyed_color=rng.randrange(1 << 24)))
    for potion in ['minecraft:water', 'minecraft:healing', 'minecraft:awkward']:
        for item in ['potion', 'splash_potion', 'lingering_potion']:
            held.append(snbt('minecraft:' + item, rng.choice([1, 2]), potion_contents='{potion:"%s"}' % potion))
    cases = []
    filler = [snbt(i, 64) for i in ['minecraft:stone', 'minecraft:dirt', 'minecraft:cobblestone']]
    for state in STATES:
        for stack in held:
            crowded = rng.random() < 0.25
            inventory = [[i, rng.choice(filler)] for i in range(1, 36)] if crowded else []
            if not crowded and rng.random() < 0.3:
                inventory = [[rng.randrange(1, 36), snbt('minecraft:bucket', rng.randrange(1, 16))]]
            cases.append({'state': state, 'held': stack, 'creative': rng.random() < 0.3, 'under_water': rng.random() < 0.3,
                          'inventory': inventory})
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    vanilla.run('VanillaCauldronOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'cauldron_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')
    total = lib.compare(cases, work / 'native.txt', work / 'rust.txt',
                        lambda case, line: case['state'].split('|')[0] + ' ' + line.split('\t')[0],
                        lambda case: json.dumps(case)[:700], args.limit)
    raise SystemExit(1 if total else 0)


if __name__ == '__main__':
    main()
