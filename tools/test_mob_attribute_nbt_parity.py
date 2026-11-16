#!/usr/bin/env python3
"""Compare a mob's and an armor stand's saved `attributes` (a base, a permanent modifier with its id) and
the item modifiers of LivingEntity.collectEquipmentChanges with vanilla.

Each case loads an entity from NBT, equips it, runs the equipment pass (twice for a swap) and prints the
attribute values and bases asked for plus the saved `attributes` list as exact double bits. The item
modifiers are transient, so they show in the values and never in the saved list."""
import argparse
import json
import os
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

POS = 'Pos:[0.0d,64.0d,0.0d],Motion:[0.0d,0.0d,0.0d],Rotation:[0.0f,0.0f]'
BONUS = '{id:"test:bonus",amount:0.5d,operation:"add_multiplied_base"}'


def case(name, entity, attributes, equip=None, then=None, read=()):
    snbt = f'{{id:"{entity}",{POS},Health:20.0f'
    if attributes:
        snbt += f',attributes:[{",".join(attributes)}]'
    return dict(name=name, entity=entity, snbt=snbt + '}', equip=equip or {}, then=then or {}, read=list(read))


ZOMBIE, STAND = 'minecraft:zombie', 'minecraft:armor_stand'
CASES = [
    case('zombie_defaults', ZOMBIE, [], read=['minecraft:max_health', 'minecraft:attack_damage', 'minecraft:armor']),
    # A base override and a non-transient modifier with an id on a modelled attribute, a base and a
    # modifier on one the game does not model (step_height), and an attribute the type lacks (luck).
    case('zombie_saved', ZOMBIE, [
        f'{{id:"minecraft:max_health",base:40.0d,modifiers:[{BONUS}]}}',
        '{id:"minecraft:attack_damage",base:5.0d}',
        '{id:"minecraft:step_height",base:1.5d,modifiers:[{id:"test:step",amount:0.25d,operation:"add_value"}]}',
        '{id:"minecraft:luck",base:2.0d}',
    ], read=['minecraft:max_health', 'minecraft:attack_damage', 'minecraft:step_height', 'minecraft:luck']),
    case('zombie_sword', ZOMBIE, [], equip={'mainhand': 'minecraft:diamond_sword'}, read=['minecraft:attack_damage']),
    case('zombie_sword_swapped', ZOMBIE, ['{id:"minecraft:attack_damage",base:5.0d}'],
         equip={'mainhand': 'minecraft:diamond_sword'}, then={'mainhand': 'minecraft:netherite_sword'},
         read=['minecraft:attack_damage']),
    case('zombie_sword_put_away', ZOMBIE, [], equip={'mainhand': 'minecraft:diamond_sword'}, then={'mainhand': ''},
         read=['minecraft:attack_damage']),
    case('zombie_armor', ZOMBIE, [f'{{id:"minecraft:max_health",base:40.0d,modifiers:[{BONUS}]}}'], equip={
        'head': 'minecraft:iron_helmet', 'chest': 'minecraft:iron_chestplate', 'feet': 'minecraft:netherite_boots',
        'offhand': 'minecraft:diamond_sword',
    }, read=['minecraft:armor', 'minecraft:armor_toughness', 'minecraft:knockback_resistance', 'minecraft:attack_damage']),
    case('zombie_sword_on_head', ZOMBIE, [], equip={'head': 'minecraft:diamond_sword'}, read=['minecraft:attack_damage']),
    # A ticked stand has the movement_speed instance LivingEntity.removeFrost makes on its first tick.
    case('stand_ticked', STAND, ['{id:"minecraft:movement_speed",base:0.7d}'], read=['minecraft:max_health', 'minecraft:armor', 'minecraft:attack_damage']),
    case('stand_saved', STAND, [
        f'{{id:"minecraft:max_health",base:30.0d,modifiers:[{BONUS}]}}',
        '{id:"minecraft:movement_speed",base:0.25d}',
    ], equip={'head': 'minecraft:diamond_helmet'}, read=['minecraft:max_health', 'minecraft:armor', 'minecraft:movement_speed']),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-mob-attribute-nbt-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobAttributeNbtOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))
    cases = build / 'cases.jsonl'
    cases.write_text(''.join(json.dumps(c) + '\n' for c in CASES), encoding='utf-8')

    def run(cmd, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            subprocess.run(list(map(str, cmd)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]), 'dev.horizonlab.parity.VanillaMobAttributeNbtOracle', cases, build / 'vanilla.txt'], 'java')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'fast/examples/client_probe'
    run([binary, 'mob_attribute_nbt_fixture', lab.RESOURCES, cases, build / 'rust.txt'], 'rust')
    vanilla = build.joinpath('vanilla.txt').read_text(encoding='utf-8').splitlines()
    rust = build.joinpath('rust.txt').read_text(encoding='utf-8').splitlines()
    problems = []
    for v, r in zip(vanilla, rust):
        if v == r:
            continue
        v, r = v.split('\t'), r.split('\t')
        problems.append(f'{v[0]}:')
        problems += [f'  vanilla only: {x}' for x in v[1:] if x not in r[1:]]
        problems += [f'  rust only:    {x}' for x in r[1:] if x not in v[1:]]
    if len(vanilla) != len(rust):
        problems.append(f'vanilla has {len(vanilla)} cases, rust {len(rust)}')
    if problems:
        print('\n'.join(problems))
        raise SystemExit('mob attribute NBT divergences')
    print(f'Mob attribute NBT parity passed ({len(CASES)} cases)')


if __name__ == '__main__':
    main()
