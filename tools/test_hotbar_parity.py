#!/usr/bin/env python3
"""Compare creative toolbars read from an older hotbar.nbt with the vanilla client's.

Each case is the SNBT of a hotbar.nbt of some DataVersion (the version tag
missing means 1343): nine rows of nine stacks in the item format of that era
(damage values of 1.12, `tag` compounds of 1.13 to 1.20.4, components of
1.20.5 on), with empty stacks (air, count zero or negative, no
id), rows of the wrong size or type and garbage. The Java oracle
(VanillaHotbarOracle) writes the file and loads it with the client's real
HotbarManager and DataFixers (DataFixTypes.HOTBAR, so EmptyItemInHotbarFix
and every item stack fixer), then Hotbar.load on each row;
examples/probe/hotbar_fixture runs horizon_client::hotbar on the same tag. The nine
rows' stacks are compared.
"""
import argparse
import json
import os
import random
import subprocess
import sys
try:
    from . import interaction_lab as lib, test_model_baking_parity as baking, test_rust_parity as base
except ImportError:
    import interaction_lab as lib
    import test_model_baking_parity as baking
    import test_rust_parity as base

VERSIONS = [None, 1343, 1344, 1451, 1519, 1631, 1976, 2200, 2504, 2586, 2730, 2975, 3105, 3218, 3337, 3465, 3578, 3698,
            3700, 3817, 3818, 3820, 3824, 3825, 3900, 4059, 4082, 4189, 4325, 4540, 4650, 4780, 4900, 5023]

LEGACY = [
    '{id:"minecraft:planks",Damage:2s,Count:3b}', '{id:"minecraft:wool",Damage:14s,Count:64b}',
    '{id:"minecraft:stone",Damage:3s,Count:1b}', '{id:"minecraft:log",Damage:1s,Count:1b}',
    '{id:"minecraft:log2",Damage:1s,Count:7b}', '{id:"minecraft:potion",Damage:8193s,Count:1b}',
    '{id:"minecraft:diamond_pickaxe",Damage:12s,Count:1b,tag:{ench:[{id:32s,lvl:3s}],display:{Name:"Pick"}}}',
    '{id:"minecraft:golden_apple",Damage:1s,Count:2b}', '{id:"minecraft:dye",Damage:4s,Count:5b}',
    '{id:"minecraft:stained_glass_pane",Damage:5s,Count:9b}', '{id:"minecraft:skull",Damage:3s,Count:1b,tag:{SkullOwner:"Notch"}}',
    '{id:"minecraft:banner",Damage:1s,Count:1b,tag:{BlockEntityTag:{Base:1,Patterns:[{Pattern:"cre",Color:15}]}}}',
    '{id:"minecraft:tallgrass",Damage:1s,Count:1b}', '{id:"minecraft:stick",Count:1b}', '{id:"minecraft:iron_sword",Count:1b,Damage:40s}',
    '{id:"minecraft:bed",Damage:14s,Count:1b}', '{id:"minecraft:monster_egg",Damage:2s,Count:3b}',
    '{id:"minecraft:written_book",Count:1b,tag:{title:"t",author:"a",pages:["{\\"text\\":\\"p\\"}"]}}',
]
CLASSIC = [
    '{id:"minecraft:diamond_sword",Count:1b,tag:{Damage:5,Enchantments:[{id:"minecraft:sharpness",lvl:3s}],display:{Name:\'{"text":"Cool"}\'}}}',
    '{id:"minecraft:potion",Count:1b,tag:{Potion:"minecraft:strong_healing"}}', '{id:"minecraft:stone",Count:64b}',
    '{id:"minecraft:firework_rocket",Count:3b,tag:{Fireworks:{Flight:2b}}}', '{id:"minecraft:cobblestone",Count:12b}',
    '{id:"minecraft:player_head",Count:1b,tag:{SkullOwner:"Notch"}}', '{id:"minecraft:dirt",Count:2b}',
    '{id:"minecraft:white_banner",Count:1b,tag:{BlockEntityTag:{Patterns:[{Pattern:"cre",Color:15}]}}}',
    '{id:"minecraft:enchanted_book",Count:1b,tag:{StoredEnchantments:[{id:"minecraft:mending",lvl:1s}]}}',
    '{id:"minecraft:tipped_arrow",Count:8b,tag:{Potion:"minecraft:weakness"}}', '{id:"minecraft:string",Count:3b}',
    '{id:"minecraft:iron_axe",Count:1b,tag:{Damage:3,RepairCost:7,Unbreakable:1b}}',
    '{id:"minecraft:filled_map",Count:1b,tag:{map:12}}', '{id:"minecraft:leather_chestplate",Count:1b,tag:{display:{color:255}}}',
    '{id:"minecraft:shield",Count:1b,tag:{BlockEntityTag:{Base:3}}}', '{id:"minecraft:bundle",Count:1b,tag:{Items:[{id:"minecraft:stone",Count:2b}]}}',
    '{id:"minecraft:diamond_sword",Count:1b,tag:{AttributeModifiers:[{AttributeName:"generic.attack_damage",Name:"x",Amount:2.5d,Operation:0,UUID:[I;1,2,3,4],Slot:"mainhand"}]}}',
]
COMPONENTS = [
    '{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5}}', '{id:"minecraft:stone",count:64}',
    '{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}',
    '{id:"minecraft:firework_rocket",count:3,components:{"minecraft:fireworks":{flight_duration:2b}}}',
    '{id:"minecraft:white_banner",count:1}', '{id:"minecraft:iron_axe",count:1,components:{"minecraft:repair_cost":7}}',
    '{id:"minecraft:grass_block",count:2}', '{id:"minecraft:cobblestone",count:12}', '{id:"minecraft:stone"}',
]
JUNK = ['{}', '{id:"minecraft:air",Count:1b}', '{id:"minecraft:stone",Count:0b}', '{id:"minecraft:stone",Count:-3b}',
        '{Count:5b}', '{id:"minecraft:stone"}', '{id:"minecraft:air",count:3}', '{id:"minecraft:stone",count:0}',
        '{id:"minecraft:stone",count:200}', '{id:"minecraft:stone",Count:100b}', '{id:"minecraft:nonexistent",Count:1b}',
        '{id:5s,Count:1b}', '{id:"minecraft:stone",Count:"x"}']
# Entries that are no stacks, and rows that are no lists: the fixers drop the entry from its row (so a row of ten
# with one is a full row, and a row of nine a short one) and a row that is no list ends the reading. Several of
# them in one file interact in ways this does not follow, so each gets its own file.
ODD = ['"stone"', '5', '[]', '1b', '{}', '[I;1,2]']


def pool(version):
    v = version or 1343
    if v < 1451:
        return LEGACY
    if v < 3819:
        return CLASSIC
    return COMPONENTS


def stack(rng, version):
    r = rng.random()
    if r < 0.75:
        return rng.choice(pool(version))
    if r < 0.92:
        return rng.choice(JUNK)
    return rng.choice(pool(rng.choice(VERSIONS)))


def case(rng):
    version = rng.choice(VERSIONS)
    rows = []
    elements = {}
    for i in range(9):
        r = rng.random()
        if r < 0.04:
            continue
        if r < 0.07:
            rows.append(f'"{i}":5')
            continue
        size = 9 if r < 0.93 else rng.choice([0, 8, 10])
        stacks = [stack(rng, version) for _ in range(size)]
        elements[i] = stacks
        rows.append(f'"{i}":[' + ','.join(stacks) + ']')
    head = '' if version is None else f'DataVersion:{version},'
    return {'snbt': '{' + head + ','.join(rows) + '}', 'version': version, 'elements': elements}


def odd_cases():
    cases = []
    for version in [None, 1343, 1631, 3337, 3700, 3818, 3819, 3824, 4325, 4540, 5015, 5016, 5022, 5023]:
        v = version or 1343
        ok = '{id:"minecraft:stone",Count:2b}' if v < 3819 else '{id:"minecraft:stone",count:2}'
        head = '' if version is None else f'DataVersion:{version},'
        good = '[' + ','.join([ok] * 9) + ']'
        for odd in ODD:
            for position in (0, 4, 8):
                row = [ok] * 9
                row[position] = odd
                cases.append({'snbt': '{' + head + f'"0":[{",".join(row)}],"1":{good}' + '}'})
            for row_size in (8, 10):
                row = [ok] * row_size
                row[3] = odd
                cases.append({'snbt': '{' + head + f'"0":[{",".join(row)}],"1":{good}' + '}'})
            # A row that is none: the rows after it.
            cases.append({'snbt': '{' + head + ','.join(f'"{i}":{odd if i == 2 else good}' for i in range(9)) + '}'})
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=23)
    parser.add_argument('--cases', type=int, default=400)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    lock, jars, _ = base.verify_artifacts()
    work = lib.work_dir('hotbar-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    done = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-cp', deps,
                           '-d', str(classes), *(str(lib.PARITY / f'{n}.java') for n in ('ParityData', 'VanillaHotbarOracle'))],
                          capture_output=True, text=True)
    if done.returncode:
        print(done.stderr[-4000:])
        sys.exit('javac failed')
    cases = [{'snbt': None}] + odd_cases() + [case(rng) for _ in range(args.cases)]
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    run = subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + deps, 'dev.horizonlab.parity.VanillaHotbarOracle',
                          str(work / 'cases.jsonl'), str(work / 'native.txt')], capture_output=True, text=True)
    if run.returncode:
        print(run.stderr[-4000:])
        sys.exit('oracle failed')
    lib.cargo_example('horizon-client', 'hotbar_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')
    bad = lib.compare(cases, work / 'native.txt', work / 'rust.txt', lambda c, line: 'hotbar' if not line.startswith('error') else 'oracle error',
                      describe=lambda c: c['snbt'][:300] if c['snbt'] else 'missing file', limit=args.limit)
    raise SystemExit(1 if bad else 0)


if __name__ == '__main__':
    main()
