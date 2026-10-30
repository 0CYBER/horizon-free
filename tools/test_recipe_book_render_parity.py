#!/usr/bin/env python3
"""Compare the recipe book's drawing with the vanilla client's.

Each case opens a book type (the inventory's or a crafting table's crafting
book, the furnace, blast furnace or smoker) at a GUI size, with its visible
tabs (one selected, some in their new-recipe squeeze), the search box (text,
cursor, selection, focus and cursor blink), the craftable filter, a page of
recipe collections (real recipes, some craftable, some buttons squeezing),
the alternatives overlay of one button, ghost slots, the cycling index and
the mouse. The Java oracle (VanillaRecipeBookScreenOracle) builds vanilla's
widgets for that state as RecipeBookComponent.initVisuals and
RecipeBookPage.init do and records every GuiGraphicsExtractor call of their
extractRenderState, the tooltips and GhostSlots.extractRenderState, with its
pose; examples/probe/recipe_book_render_fixture lays out the same state with
horizon_client::recipe_book_screen, which the game paints. The call lists
must agree.
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

TABS = {'crafting': 5, 'furnace': 4, 'blast_furnace': 3, 'smoker': 2}
TYPES = {'crafting': ('minecraft:crafting_shaped', 'minecraft:crafting_shapeless'), 'furnace': ('minecraft:smelting',),
         'blast_furnace': ('minecraft:blasting',), 'smoker': ('minecraft:smoking',)}
WORDS = ['', '', 'a', 'stone', 'iron ingot', 'oak', 'zzz', 'red', 'a very long search text that overflows']


def recipes_by_type():
    out = {k: [] for k in TYPES}
    for path in sorted((lib.RESOURCES / 'data/minecraft/recipe').glob('*.json')):
        kind = json.loads(path.read_text()).get('type')
        for k, types in TYPES.items():
            if kind in types:
                out[k].append('minecraft:' + path.stem)
    return out


def cases(count, seed):
    rng = random.Random(seed)
    by_type = recipes_by_type()
    items = sorted(i for i in json.loads((lib.RESOURCES / 'items.json').read_text()) if i != 'minecraft:air')
    out = []
    for n in range(count):
        kind = ['crafting', 'crafting', 'furnace', 'blast_furnace', 'smoker'][n % 5]
        width, height = rng.choice([(427, 240), (480, 270), (640, 360), (320, 240), (378, 260), (854, 480)])
        narrow = width < 379
        xo = (width - 147) // 2 - (0 if narrow else 86)
        yo = (height - 166) // 2
        tabs = [0] + sorted(rng.sample(range(1, TABS[kind]), rng.randint(0, TABS[kind] - 1)))
        selected = rng.choice(tabs)
        tab_list = [{'tab': t, 'selected': t == selected, 'anim': rng.choice([0.0, 0.0, 0.0, round(rng.uniform(0.1, 15), 3)])}
                    for t in tabs]
        total = rng.choice([0, 1, 5, 19, 20, 21, 47, 80])
        pages = (total + 19) // 20
        page = rng.randrange(pages) if pages else 0
        filtering = rng.random() < 0.4
        collections = []
        for _ in range(total):
            group = rng.sample(by_type[kind], rng.choice([1, 1, 1, 2, 3, 6]))
            recipes = [[r, rng.random() < 0.4] for r in group]
            if filtering and not any(c for _, c in recipes):
                recipes[0][1] = True
            collections.append({'recipes': recipes})
        on_page = max(0, min(20, total - 20 * page))
        value = rng.choice(WORDS)
        cursor = rng.randint(0, len(value))
        highlight = rng.choice([cursor, rng.randint(0, len(value))])
        search = {'value': value, 'cursor': cursor, 'highlight': highlight,
                  'display': rng.choice([0, 0, min(cursor, rng.randint(0, len(value)))]), 'focused': rng.random() < 0.5,
                  'blink': rng.choice([0, 150, 450, 700])}
        targets = [(xo + 11 + 25 * (i % 5) + rng.randint(0, 24), yo + 31 + 25 * (i // 5) + rng.randint(0, 24)) for i in range(on_page)]
        targets += [(xo + 115, yo + 20), (xo + 95, yo + 140), (xo + 40, yo + 140), (xo + 30, yo + 18), (xo - 20, yo + 40),
                    (rng.randint(0, width), rng.randint(0, height))]
        case = {'type': kind, 'grid': 2 if rng.random() < 0.5 else 3, 'width': width, 'height': height, 'narrow': narrow,
                'mouse': list(rng.choice(targets)), 'index': rng.choice([0, 0, 1, 2, 5, 13, 60]), 'filtering': filtering,
                'tabs': tab_list, 'search': search, 'collections': collections, 'page': page,
                'buttonAnims': [rng.choice([0.0, 0.0, 0.0, round(rng.uniform(0.1, 15), 3)]) for _ in range(on_page)]}
        multi = [i for i in range(on_page) if sum(1 for _, c in collections[20 * page + i]['recipes'] if c or not filtering) > 1]
        if multi and rng.random() < 0.4:
            case['overlay'] = rng.choice(multi)
        if rng.random() < 0.4:
            left, top = (width - 176) // 2, (height - 166) // 2
            slots = [(124, 35, True)] + [(30 + 18 * (i % 3), 17 + 18 * (i // 3), False) for i in range(9)]
            if kind != 'crafting':
                slots = [(116, 35, True), (56, 17, False)]
            case.update({'left': left, 'top': top, 'big': rng.random() < 0.7,
                         'ghost': [{'x': x, 'y': y, 'result': r, 'items': rng.sample(items, rng.choice([1, 1, 2, 4]))}
                                   for x, y, r in rng.sample(slots, rng.randint(1, len(slots)))]})
        out.append(case)
    return out


def same(a, b):
    """Calls agree: every field exactly, poses within float printing."""
    if a is None or b is None or set(a) != set(b):
        return False
    return all(all(abs(x - y) <= 1e-3 for x, y in zip(a[k], b[k])) if k == 'm' else a[k] == b[k] for k in a)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=29)
    parser.add_argument('--cases', type=int, default=600)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    work = lib.work_dir('recipe-book-render-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    done = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-cp', deps,
                           '-d', str(classes), *(str(lib.PARITY / f'{n}.java') for n in ('ParityData', 'ParityFont', 'VanillaRecipeBookScreenOracle'))],
                          capture_output=True, text=True)
    if done.returncode:
        print(done.stderr[-4000:])
        sys.exit('javac failed')
    chosen = cases(args.cases, args.seed)
    inputs = work / 'cases.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    run = subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + deps, 'dev.horizonlab.parity.VanillaRecipeBookScreenOracle',
                          str(inputs), str(work / 'vanilla.jsonl')], capture_output=True, text=True)
    if run.returncode:
        print(run.stderr[-4000:])
        sys.exit('oracle failed')
    lib.cargo_example('horizon-client', 'recipe_book_render_fixture', lib.RESOURCES, inputs, work / 'rust.jsonl')
    vanilla = [json.loads(l) for l in (work / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (work / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    errors, failures, passed = {}, {}, 0
    for case, v, r in zip(chosen, vanilla, rust):
        if 'error' in v:
            errors.setdefault(case['type'], v['error'])
            continue
        a, b = v['draws'], r['draws']
        bad = next((i for i in range(max(len(a), len(b))) if i >= len(a) or i >= len(b) or not same(a[i], b[i])), None)
        if bad is None:
            passed += 1
        else:
            failures.setdefault(case['type'], []).append((case, bad, a[bad] if bad < len(a) else None, b[bad] if bad < len(b) else None))
    for kind, error in sorted(errors.items()):
        print(f'ORACLE ERROR {kind}: {error[:1500]}')
    for kind, items in sorted(failures.items()):
        print(f'MISMATCH {kind}: {len(items)} cases')
        for case, i, x, y in items[:args.limit]:
            brief = {k: v for k, v in case.items() if k not in ('collections',)}
            print(f'    {json.dumps(brief)[:700]}')
            print(f'      call {i}: vanilla {json.dumps(x)}')
            print(f'      call {i}: horizon {json.dumps(y)}')
    print(f'{passed}/{len(chosen)} cases match')
    if errors or failures:
        sys.exit(1)
    print('recipe book render parity ok')


if __name__ == '__main__':
    main()
