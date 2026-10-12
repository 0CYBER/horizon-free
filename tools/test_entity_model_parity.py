#!/usr/bin/env python3
"""Compare entity model geometry and setupAnim poses with the vanilla 26.3 client.

The Java oracle bakes each vanilla model layer, builds the model's own render
state class with the generated walk/age/head/attack inputs (plus per-species
state fields), runs the real `setupAnim` and writes every part's pose.
`examples/entity_anim_fixture` replays the same inputs through Horizon's
`mobs::anim::setup` and the loaded model tree; the two must agree on every part
pose and on the baked polygon count and vertex sums.

  test_entity_model_parity.py [--filter SUBSTRING] [--detail LAYER] [--tolerance EPS]
"""
import argparse
import json
import math
import os
from pathlib import Path
import random
import re
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

CLIENT = lab.LOCAL / 'minecraft' / '26.3' / 'sources' / 'client' / 'net' / 'minecraft' / 'client'

# Models that no renderer builds with a plain `new Model(context.bakeLayer(CONSTANT))`.
EXTRAS = [
    ('minecraft:zombie#main', 'ZombieModel'), ('minecraft:zombie_baby#main', 'BabyZombieModel'),
    ('minecraft:husk#main', 'ZombieModel'), ('minecraft:husk_baby#main', 'BabyZombieModel'),
    ('minecraft:zombie_villager#main', 'ZombieVillagerModel'),
    ('minecraft:zombie_villager_baby#main', 'BabyZombieVillagerModel'),
    ('minecraft:zombified_piglin#main', 'AdultZombifiedPiglinModel'),
    ('minecraft:zombified_piglin_baby#main', 'BabyZombifiedPiglinModel'),
    ('minecraft:skeleton#main', 'SkeletonModel'), ('minecraft:stray#main', 'SkeletonModel'),
    ('minecraft:wither_skeleton#main', 'SkeletonModel'),
    ('minecraft:spider#main', 'SpiderModel'), ('minecraft:cave_spider#main', 'SpiderModel'),
    ('minecraft:enderman#main', 'EndermanModel'), ('minecraft:endermite#main', 'EndermiteModel'),
    ('minecraft:piglin#main', 'AdultPiglinModel'), ('minecraft:piglin_brute#main', 'AdultPiglinModel'),
    ('minecraft:piglin_baby#main', 'BabyPiglinModel'),
    ('minecraft:hoglin#main', 'HoglinModel'), ('minecraft:zoglin#main', 'HoglinModel'),
    ('minecraft:hoglin_baby#main', 'BabyHoglinModel'), ('minecraft:zoglin_baby#main', 'BabyHoglinModel'),
    ('minecraft:guardian#main', 'GuardianModel'), ('minecraft:elder_guardian#main', 'GuardianModel'),
    ('minecraft:evoker#main', 'IllagerModel'), ('minecraft:illusioner#main', 'IllagerModel'),
    ('minecraft:pillager#main', 'IllagerModel'), ('minecraft:vindicator#main', 'IllagerModel'),
    ('minecraft:llama#main', 'LlamaModel'), ('minecraft:trader_llama#main', 'LlamaModel'),
    ('minecraft:llama_baby#main', 'BabyLlamaModel'), ('minecraft:trader_llama_baby#main', 'BabyLlamaModel'),
    ('minecraft:donkey#main', 'DonkeyModel'), ('minecraft:mule#main', 'DonkeyModel'),
    ('minecraft:donkey_baby#main', 'BabyDonkeyModel'), ('minecraft:mule_baby#main', 'BabyDonkeyModel'),
    ('minecraft:skeleton_horse#main', 'HorseModel'), ('minecraft:zombie_horse#main', 'HorseModel'),
    ('minecraft:skeleton_horse_baby#main', 'BabyHorseModel'), ('minecraft:zombie_horse_baby#main', 'BabyHorseModel'),
    ('minecraft:cave_spider#main', 'SpiderModel'),
    ('minecraft:stray#main', 'SkeletonModel'),
]
# Layers a renderer reuses for another entity under a different constant.
SKIP_LAYERS = re.compile(r'(banner|bell|book|skull|head|chest|shulker_box|boat|cushion|conduit|leash|arrow|trident|'
                         r'shield|elytra|spin_attack|ears|cape|decorated_pot|minecart|wind_charge|evoker_fangs|'
                         r'llama_spit|shulker_bullet|bee_stinger|end_crystal|armor_stand|copper_golem_\w+|#(helmet|chestplate|leggings|'
                         r'boots|outer|armor|decor|saddle|collar|wool|wool_undercoat|eyes|wind|inner|pattern|tendrils|'
                         r'heart|bioluminescent|pulsating_spots|water_patch|flag|cage|shell)$)')


def layer_constant(name):
    """minecraft:cow_baby#main -> constant COW_BABY (the main layer of a model)."""
    return name.split(':')[1].split('#')[0].upper()


def renderer_pairs():
    pairs = {}
    pattern = re.compile(r'new (\w+)\(\s*(?:context|this\.context|\w*)\.bakeLayer\(ModelLayers\.(\w+)\)')
    for path in sorted((CLIENT / 'renderer').rglob('*.java')):
        for match in pattern.finditer(path.read_text()):
            pairs[match.group(2)] = match.group(1)
    return pairs


def layers(models):
    chosen = {}
    pairs = renderer_pairs()
    for key in models:
        if not key.endswith('#main') or SKIP_LAYERS.search(key):
            continue
        constant = layer_constant(key)
        if constant in pairs:
            chosen[key] = pairs[constant]
    for key, cls in EXTRAS:
        if key in models:
            chosen[key] = cls
    return chosen


def cases(layer):
    rng = random.Random(0x414e494d + sum(map(ord, layer)))
    baby = '_baby#' in layer
    base_cases = [
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        (1.0, 0.5, 10.0, 0.0, 0.0, 0.0),
        (3.7, 0.6, 57.3, 20.0, -15.0, 0.0),
        (12.25, 1.0, 123.5, -35.0, 40.0, 0.0),
        (7.5, 0.3, 33.3, 5.0, 5.0, 0.35),
        (2.0, 0.0, 77.7, 45.0, -60.0, 0.8),
    ]
    for _ in range(14):
        base_cases.append((rng.uniform(0, 30), rng.uniform(0, 1), rng.uniform(0, 400),
                           rng.uniform(-50, 50), rng.uniform(-70, 70),
                           rng.choice([0.0, rng.uniform(0.01, 0.99)])))
    cases_out = []
    for pos, speed, age, y, x, attack in base_cases:
        cases_out.append({'walkAnimationPos': pos, 'walkAnimationSpeed': speed, 'ageInTicks': age,
               'yRot': y, 'xRot': x, 'swingAnimation': attack, 'isBaby': baby, 'ageScale': 0.5 if baby else 1.0,
               'entityId': 7, 'yHeadRot': y, 'yBodyRot': 0.0})
    name = layer.split(':')[1].split('#')[0].removesuffix('_baby')
    cases_out = [{**c, **DEFAULTS.get(name, {})} for c in cases_out]
    yield from cases_out
    yield from species_cases(layer, cases_out)


T, F = True, False
FOX = [{'isCrouching': T, 'crouchAmount': 0.8}, {'isSleeping': T}, {'isSitting': T}, {'isPouncing': T, 'crouchAmount': 0.6},
       {'isFaceplanted': T}, {'headRollAngle': 0.3}, {'isCrouching': T, 'isPouncing': T, 'crouchAmount': 0.4}]
ILLAGER = [{'armPose': pose, 'maxCrossbowChargeDuration': 25, 'ticksUsingItem': 10.0}
           for pose in ['NEUTRAL', 'CROSSED', 'ATTACKING', 'SPELLCASTING', 'BOW_AND_ARROW', 'CROSSBOW_HOLD',
                        'CROSSBOW_CHARGE', 'CELEBRATING']] + [{'isRiding': T, 'armPose': 'CROSSED'}]
FELINE = [{'isCrouching': T}, {'isSprinting': T}, {'isSitting': T}, {'lieDownAmount': 0.7, 'lieDownAmountTail': 0.5},
          {'relaxStateOneAmount': 0.6}, {'isSitting': T, 'lieDownAmount': 1.0, 'lieDownAmountTail': 0.2}]
ZOMBIE = [{'isAggressive': T}, {'isCrouching': T}, {'isPassenger': T}, {'isAggressive': T, 'isPassenger': T, 'isCrouching': T}]
SKELETON = ZOMBIE + [{'isAggressive': T, 'isHoldingBow': T, 'rightArmPose': 'BOW_AND_ARROW'}, {'isHoldingBow': T},
                     {'isAggressive': T, 'isHoldingBow': T, 'rightArmPose': 'BOW_AND_ARROW', 'isCrouching': T}]
PIGLIN = [{'armPose': pose, 'maxCrossbowChageDuration': 25.0, 'ticksUsingItem': 12.0}
          for pose in ['DEFAULT', 'DANCING', 'ATTACKING_WITH_MELEE_WEAPON', 'CROSSBOW_HOLD', 'CROSSBOW_CHARGE', 'ADMIRING_ITEM']]
EQUINE = [{'eatAnimation': 0.6}, {'standAnimation': 0.5}, {'eatAnimation': 0.3, 'standAnimation': 0.4, 'feedingAnimation': 0.7},
          {'isInWater': T}, {'standAnimation': 1.0, 'eatAnimation': 1.0}]
WATER = [{'isInWater': T}, {'isInWater': F}]
SPECIES = {
    'horse': EQUINE, 'donkey': EQUINE + [{'hasChest': T}], 'mule': EQUINE + [{'hasChest': T}], 'zombie_horse': EQUINE, 'skeleton_horse': EQUINE,
    'iron_golem': [{'attackTicksRemaining': 7.0}, {'offerFlowerTick': 30}, {'attackTicksRemaining': 0.0, 'offerFlowerTick': 0}],
    'hoglin': [{'attackAnimationRemainingTicks': t} for t in (3, 7, 10)],
    'zoglin': [{'attackAnimationRemainingTicks': t} for t in (3, 7, 10)],
    'ravager': [{'stunnedTicksRemaining': 20.0}, {'attackTicksRemaining': 8.0}, {'attackTicksRemaining': 3.0},
                {'roarAnimation': 0.5}],
    'vex': [{'isCharging': T}, {'isCharging': F}],
    'squid': [{'tentacleAngle': 0.7}], 'glow_squid': [{'tentacleAngle': 1.2}],
    'dolphin': [{'isMoving': T}, {'isMoving': F}],
    'cod': WATER, 'salmon': WATER, 'tropical_fish_small': WATER,
    'allay': [{'isDancing': T, 'isSpinning': T, 'spinningProgress': 0.6}, {'isDancing': T, 'isSpinning': F, 'spinningProgress': 0.2},
              {'holdingAnimationProgress': 0.8}, {'holdingAnimationProgress': 1.0}],
    'camel': [{'sitAnimationState': 2}, {'sitPoseAnimationState': 3}, {'sitUpAnimationState': 1}, {'idleAnimationState': 4},
              {'dashAnimationState': 2}, {'jumpCooldown': 20.0}],
    'chicken': [{'flap': 2.0, 'flapSpeed': 0.7}, {'flap': 5.3, 'flapSpeed': 1.0}],
    'wolf': [{'isSitting': T}, {'isAngry': T, 'tailAngle': 1.5393804}, {'shakeAnim': 0.9}, {'interest': 0.6},
             {'isSitting': T, 'shakeAnim': 0.5, 'interest': 0.3}],
    'zombie': ZOMBIE, 'husk': ZOMBIE, 'drowned': ZOMBIE, 'zombie_villager': ZOMBIE, 'zombified_piglin': ZOMBIE, 'giant': ZOMBIE,
    'skeleton': SKELETON, 'stray': SKELETON, 'wither_skeleton': SKELETON, 'bogged': SKELETON,
    'piglin': PIGLIN, 'piglin_brute': PIGLIN,
    'cat': FELINE, 'ocelot': FELINE,
    'creaking': [{'canMove': F}, {'attackAnimationState': 2}, {'invulnerabilityAnimationState': 3}, {'deathAnimationState': 1}],
    'evoker': ILLAGER, 'pillager': ILLAGER, 'vindicator': ILLAGER, 'illusioner': ILLAGER,
    'sheep': [{'eatTick': t} for t in (2, 10, 36, 39)],
    'guardian': [{'tailAnimation': 4.0, 'spikesAnimation': 0.3}, {'tailAnimation': 0.0, 'spikesAnimation': 0.0}],
    'elder_guardian': [{'tailAnimation': 4.0, 'spikesAnimation': 0.3}],
    'shulker': [{'peekAmount': v} for v in (0.0, 0.2, 0.5, 1.0)],
    'witch': [{'isHoldingItem': T}, {'isHoldingItem': F}],
    'goat': [{'hasLeftHorn': T, 'hasRightHorn': T}, {'hasLeftHorn': F, 'hasRightHorn': T, 'rammingXHeadRot': 0.6},
             {'hasLeftHorn': T, 'hasRightHorn': F}],
    'polar_bear': [{'standScale': 0.0}, {'standScale': 0.5}, {'standScale': 1.0}],
    'panda': [{'isUnhappy': T}, {'isSneezing': T, 'sneezeTime': 5}, {'isSneezing': T, 'sneezeTime': 16},
              {'sitAmount': 0.6, 'isEating': T}, {'sitAmount': 1.0, 'isScared': T}, {'sitAmount': 0.3},
              {'lieOnBackAmount': 0.5}, {'rollAmount': 0.7}],
    'llama': [{'hasChest': T}, {'hasChest': F}],
    'trader_llama': [{'hasChest': T}, {'hasChest': F}],
    'turtle': [{'isOnLand': T}, {'isOnLand': T, 'isLayingEgg': T}, {'isOnLand': F, 'hasEgg': T}, {'isOnLand': T, 'hasEgg': T}],
    'strider': [{'isRidden': T}, {'isRidden': F}],
    'parrot': [{'pose': pose, 'flapAngle': 0.4} for pose in ['STANDING', 'FLYING', 'ON_SHOULDER', 'SITTING', 'PARTY']],
    'bee': [{'hasStinger': T}, {'hasStinger': F, 'isOnGround': T}, {'isAngry': T}, {'rollAmount': 0.6, 'isAngry': T},
            {'isOnGround': T, 'hasStinger': T}],
    'bat': [{'isResting': T, 'restAnimationState': 3}, {'flyAnimationState': 2}, {'isResting': F, 'flyAnimationState': 0}],
    'armadillo': [{'isHidingInShell': T}, {'isHidingInShell': T, 'rollUpAnimationState': 2},
                  {'rollOutAnimationState': 4}, {'peekAnimationState': 1}],
    'axolotl': [{'playingDeadFactor': 0.0, 'inWaterFactor': 1.0, 'onGroundFactor': 0.0, 'movingFactor': 1.0},
                {'inWaterFactor': 1.0, 'movingFactor': 0.0}, {'onGroundFactor': 1.0, 'movingFactor': 1.0},
                {'onGroundFactor': 1.0, 'movingFactor': 0.0}, {'inWaterFactor': 0.5, 'onGroundFactor': 0.5, 'movingFactor': 0.5},
                {'playingDeadFactor': 1.0, 'inWaterFactor': 1.0},
                {'walkAnimationState': 1}, {'swimAnimation': 2}, {'idleOnGroundAnimationState': 3},
                {'idleUnderWaterAnimationState': 2}, {'idleUnderWaterOnGroundAnimationState': 1}, {'playDeadAnimationState': 4}],
    'frog': [{'isSwimming': T}, {'isSwimming': F}, {'jumpAnimationState': 2}, {'croakAnimationState': 1},
             {'tongueAnimationState': 3}, {'swimIdleAnimationState': 2, 'isSwimming': T}],
    'rabbit': [{'hopAnimationState': 2}, {'idleHeadTiltAnimationState': 1}],
    'fox': FOX,
    # CopperGolemModel: the idle head spin and the four chest interactions (started N ticks ago).
    'copper_golem': [{'idleAnimationState': 4}, {'idleAnimationState': 37}, {'interactionGetItem': 3},
                     {'interactionGetItem': 30}, {'interactionGetNoItem': 12}, {'interactionDropItem': 8},
                     {'interactionDropItem': 45}, {'interactionDropNoItem': 20}],
}


# Render-state defaults that are not zero/false in vanilla.
DEFAULTS = {'creaking': {'canMove': T}, 'wolf': {'tailAngle': 0.6283185307179586}, 'axolotl': {'inWaterFactor': 1.0}, 'bee': {'hasStinger': T}, 'sheep': {'eatTick': 0},
            'goat': {'hasLeftHorn': T, 'hasRightHorn': T}, 'parrot': {'pose': 'STANDING'},
            'phantom': {'flapOffset': 21.0}, 'guardian': {'tailAnimation': 1.3, 'spikesAnimation': 1.0},
            'elder_guardian': {'tailAnimation': 1.3, 'spikesAnimation': 1.0}}


def species_cases(layer, base_cases):
    name = layer.split(':')[1].split('#')[0].removesuffix('_baby')
    for extra in SPECIES.get(name, []):
        for index in (2, 3, 6):
            yield {**base_cases[index], **extra}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--detail', default='')
    parser.add_argument('--tolerance', type=float, default=5e-7)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-entity-model-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    models = json.loads((resources / 'entity-models.json').read_text())
    chosen = {k: v for k, v in layers(models).items() if args.filter in k}
    extra_cases = json.loads((Path(__file__).with_suffix('.cases.json')).read_text()) \
        if Path(__file__).with_suffix('.cases.json').exists() else {}
    lines = []
    for layer, cls in sorted(chosen.items()):
        for case in cases(layer):
            lines.append(f'{layer} {cls}|{json.dumps(case)}')
        for case in extra_cases.get(layer.split(':')[1].split('#')[0], []):
            lines.append(f'{layer} {cls}|{json.dumps(case)}')
    inputs = build / 'inputs.txt'; inputs.write_text('\n'.join(lines) + '\n')
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAnimOracle.java'
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaEntityAnimOracle',
         inputs, build / 'vanilla.jsonl', jars['client']], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client', '--example', 'entity_anim_fixture', '--',
         resources, build / 'vanilla.jsonl', build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust), (len(vanilla), len(rust))
    errors, skipped, mismatched, checked = {}, set(), {}, 0
    for v, r in zip(vanilla, rust):
        layer = v['layer']
        if 'error' in v:
            errors[layer] = v['error']; continue
        if 'skip' in r:
            skipped.add(layer); continue
        checked += 1
        bad = []
        for path, expected in v['parts'].items():
            got = r['parts'].get(path)
            if got is None:
                bad.append((path, 'missing in Horizon')); continue
            names = ['x', 'y', 'z', 'xRot', 'yRot', 'zRot', 'xScale', 'yScale', 'zScale', 'visible', 'polys', 'vsum']
            for i, (a, b) in enumerate(zip(expected, got)):
                if i == 11 and abs(a - b) <= 1e-3 * max(1, abs(a)):
                    continue
                if i == 9:
                    a, b = bool(a), bool(b)
                    if a != b:
                        bad.append((path, f'visible {a} != {b}'))
                    continue
                if abs(a - b) > args.tolerance * (1.0 + abs(a)):
                    bad.append((path, f'{names[i]} {a!r} != {b!r}'))
        for path in r['parts']:
            if path not in v['parts']:
                bad.append((path, 'extra in Horizon'))
        if bad:
            mismatched.setdefault(layer, []).append((v['case'], bad))
    print(f'{checked} cases compared over {len(chosen) - len(skipped) - len(errors)} layers')
    if errors:
        print('oracle errors:'); [print(' ', k, v) for k, v in sorted(errors.items())]
    if skipped:
        print('layers without a Horizon species:', ', '.join(sorted(skipped)))
    for layer, items in sorted(mismatched.items()):
        parts = {}
        for case, bad in items:
            for path, why in bad:
                parts.setdefault(path, (why, case))
        total = sum(1 for x in vanilla if x['layer'] == layer)
        print(f'MISMATCH {layer}: {len(items)}/{total} cases, {len(parts)} parts')
        detailed = args.detail == layer
        for path, (why, case) in list(parts.items())[:(10 ** 6 if detailed else 4)]:
            print(f'    {path or "<root>"}: {why}' + (f'   case={json.dumps(case)}' if detailed else ''))
    if mismatched or errors:
        sys.exit(f'{len(mismatched)} layers differ from vanilla')
    print('entity model parity ok')


if __name__ == '__main__':
    main()
