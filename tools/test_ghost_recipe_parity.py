#!/usr/bin/env python3
"""Compare the ghost recipe of a recipe book click with the vanilla client's.

Each case opens the inventory (2x2), a crafting table (3x3), a furnace, a
blast furnace or a smoker and clicks a recipe of its book (every crafting and
cooking recipe, the furnaces with their fuel slot empty or holding coal). The
Java oracle (VanillaGhostRecipeOracle) runs vanilla's
CraftingRecipeBookComponent / FurnaceRecipeBookComponent.fillGhostRecipe
against the real menu for every display of the recipe and prints the GhostSlots
it fills: the menu slot, whether it is the result and the stacks the slot
cycles through (PlaceRecipeHelper's centred placement of shaped recipes, the
first slots for shapeless ones, every fuel for an empty fuel slot);
examples/probe/ghost_recipe_fixture prints the same from
horizon_client::recipe_book::Book::ghost, which the game fills its ghost
slots from. The lines must agree.
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

CRAFTING = ('minecraft:crafting_shaped', 'minecraft:crafting_shapeless', 'minecraft:crafting_transmute',
            'minecraft:crafting_dye', 'minecraft:crafting_imbue')
COOKING = {'minecraft:smelting': 'furnace', 'minecraft:blasting': 'blast_furnace', 'minecraft:smoking': 'smoker'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=11)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    lock, jars, _ = base.verify_artifacts()
    work = lib.work_dir('ghost-recipe-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    sources = ('ParityData', 'VanillaMenuOracle', 'VanillaGhostRecipeOracle')
    done = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-cp', deps,
                           '-d', str(classes), *(str(lib.PARITY / f'{n}.java') for n in sources)], capture_output=True, text=True)
    if done.returncode:
        print(done.stderr[-4000:])
        sys.exit('javac failed')
    cases = []
    for path in sorted((lib.RESOURCES / 'data/minecraft/recipe').glob('*.json')):
        kind = json.loads(path.read_text()).get('type')
        recipe = 'minecraft:' + path.stem
        if kind in CRAFTING:
            for menu in ('inventory', 'crafting'):
                cases.append({'menu': menu, 'recipe': recipe})
        elif kind in COOKING:
            for fuel in (None, '{id:"minecraft:coal",count:3}'):
                cases.append({'menu': COOKING[kind], 'recipe': recipe, 'fuel': fuel})
    rng.shuffle(cases)
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    run = subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + deps, 'dev.horizonlab.parity.VanillaGhostRecipeOracle',
                          str(work / 'cases.jsonl'), str(work / 'native.txt')], capture_output=True, text=True)
    if run.returncode:
        print(run.stderr[-4000:])
        sys.exit('oracle failed')
    lib.cargo_example('horizon-client', 'ghost_recipe_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')
    bad = lib.compare(cases, work / 'native.txt', work / 'rust.txt',
                      lambda case, line: case['menu'] if not line.startswith('error') else 'oracle error',
                      describe=json.dumps, limit=args.limit)
    raise SystemExit(1 if bad else 0)


if __name__ == '__main__':
    main()
