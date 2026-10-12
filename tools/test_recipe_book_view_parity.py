#!/usr/bin/env python3
"""Compare what the recipe book lists with the vanilla client's.

Each case sends the client recipe book a history of displays as the server
does (the initial replacing batch in some order, later unlocks and
removals), opens a book type (the inventory's 2x2 or a crafting table's 3x3
grid, the furnace, blast furnace or smoker) with random inventory and menu
contents and the craftable filter on or off. The Java oracle
(VanillaRecipeBookViewOracle, on the client jar) runs ClientRecipeBook,
RecipeCollection.selectRecipes and the tab and page logic of
RecipeBookComponent; examples/recipe_book_view_fixture runs
horizon_client::recipe_book::ClientBook. Per tab: visibility, pages and
every button's displays with their selected and craftable flags, in order
(the client's HashMap of display ids decides it).
"""
import argparse
import json
import os
import random
import subprocess
import sys
try:
    from . import interaction_lab as lib, lab, test_place_recipe_parity as place, test_model_baking_parity as baking
    from . import test_crafter_parity as crafter, test_rust_parity as base
except ImportError:
    import interaction_lab as lib
    import lab
    import test_place_recipe_parity as place
    import test_model_baking_parity as baking
    import test_crafter_parity as crafter
    import test_rust_parity as base


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=17)
    parser.add_argument('--cases', type=int, default=1200)
    parser.add_argument('--limit', type=int, default=2)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    work = lib.work_dir('recipe-book-view-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    done = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-cp', deps,
                           '-d', str(classes), *(str(lib.PARITY / f'{n}.java') for n in ('ParityData', 'VanillaRecipeBookViewOracle'))],
                          capture_output=True, text=True)
    if done.returncode:
        print(done.stderr[-4000:])
        sys.exit('javac failed')
    items = json.loads((lib.RESOURCES / 'items.json').read_text())
    recipes = crafter.Recipes(lib.RESOURCES)
    gen = place.Gen(random.Random(args.seed), items, recipes)
    rng = gen.rng
    ids = sorted('minecraft:' + n for n in recipes.recipes)
    cases = []
    for n in range(args.cases):
        kind = ['crafting', 'crafting', 'furnace', 'blast_furnace', 'smoker'][n % 5]
        # Few to most recipes known: the table grows and display ids collide.
        known = rng.sample(ids, rng.choice([5, 30, 100, 300, 600, 900, len(ids)]))
        history = [{'op': 'replace', 'ids': known}]
        for _ in range(rng.choice([0, 0, 1, 2, 3])):
            if rng.random() < 0.6:
                history.append({'op': 'add', 'ids': rng.sample(ids, rng.choice([1, 3, 20]))})
            else:
                history.append({'op': 'remove', 'ids': rng.sample(known, min(len(known), rng.choice([1, 5, 40])))})
        if rng.random() < 0.1:
            history.append({'op': 'replace', 'ids': rng.sample(ids, 50)})
        menu = {'crafting': rng.choice(['inventory', 'crafting']), 'furnace': 'furnace', 'blast_furnace': 'blast_furnace',
                'smoker': 'smoker'}[kind]
        case = gen.case(menu)
        inventory = [s for _, s in case['inventory']]
        slots = [s for _, s in case['grid']]
        cases.append({'type': kind, 'grid': 2 if menu == 'inventory' else 3, 'history': history, 'inventory': inventory,
                      'slots': slots, 'filtering': rng.random() < 0.5})
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    run = subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + deps, 'dev.horizonlab.parity.VanillaRecipeBookViewOracle',
                          str(work / 'cases.jsonl'), str(work / 'native.txt')], capture_output=True, text=True)
    if run.returncode:
        print(run.stderr[-4000:])
        sys.exit('oracle failed')
    lib.cargo_example('horizon-client', 'recipe_book_view_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')

    def describe(case):
        return json.dumps({k: (v if k != 'history' else [(h['op'], len(h['ids'])) for h in v]) for k, v in case.items()
                           if k not in ('inventory', 'slots')})
    bad = lib.compare(cases, work / 'native.txt', work / 'rust.txt', lambda case, line: case['type'] if not line.startswith('error')
                      else 'oracle error', describe=describe, limit=args.limit)
    raise SystemExit(1 if bad else 0)


if __name__ == '__main__':
    main()
