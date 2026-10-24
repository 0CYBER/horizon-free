#!/usr/bin/env python3
"""Compare the loom and crafter screens' drawing with the vanilla client's.

The Java oracle (VanillaLoomScreenOracle) runs LoomScreen.extractBackground on
a loom screen and menu whose state comes from the case (inputs present, the
selectable patterns, the selected one, the scroller, the result banner or
the six-layer error, the mouse over a pattern button) and CrafterScreen's
background, disabled slots and redstone arrow, recording every
GuiGraphicsExtractor call; examples/probe/screen_layout_fixture lays out the same
state with horizon_client::screen_layout, which the game paints. The ordered
call lists must agree.
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

DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple',
        'blue', 'brown', 'green', 'red', 'black']


def no_item_required():
    root = lib.RESOURCES / 'data/minecraft/tags/banner_pattern'
    out = []
    for v in json.loads((root / 'no_item_required.json').read_text())['values']:
        out.append(v)
    return out


def cases(count, seed):
    rng = random.Random(seed)
    common = no_item_required()
    special = sorted('minecraft:' + p.stem for p in (lib.RESOURCES / 'data/minecraft/banner_pattern').glob('*.json'))
    out = []
    for n in range(count):
        width = rng.choice([320, 427, 480, 640, 961, 175])
        height = rng.choice([240, 255, 360, 501, 165])
        left, top = (width - 176) // 2, (height - 166) // 2
        if n % 4 == 3:
            out.append({'screen': 'crafter', 'width': width, 'height': height, 'left': left, 'top': top,
                        'disabled': [rng.random() < 0.3 for _ in range(9)], 'powered': rng.random() < 0.5})
            continue
        patterns = rng.choice([common, common, [rng.choice(special)], rng.sample(special, 3), []])
        rows = -(-len(patterns) // 4)
        scroll = rng.choice([0.0, 0.0, 1.0, round(rng.random(), 4)])
        start = max(int(scroll * max(rows - 4, 0) + 0.5), 0) if rng.random() < 0.8 else rng.randrange(0, max(rows, 1))
        case = {'screen': 'loom', 'width': width, 'height': height, 'left': left, 'top': top,
                'banner': rng.choice(['minecraft:white_banner', 'minecraft:red_banner', None]),
                'dye': rng.choice(['minecraft:red_dye', 'minecraft:blue_dye', None]),
                'pattern': rng.choice(['minecraft:creeper_banner_pattern', None, None]),
                'patterns': patterns, 'selected': rng.choice([-1, 0, 1, 5, 20, len(patterns) - 1]),
                'display': rng.random() < 0.75, 'hasMax': rng.random() < 0.15, 'scroll': scroll, 'startRow': start,
                'dyeStack': rng.choice(['minecraft:%s_dye' % rng.choice(DYES), None]),
                'mouseX': left + 60 + rng.randrange(-3, 60), 'mouseY': top + 13 + rng.randrange(-3, 60)}
        if rng.random() < 0.6:
            case['result'] = 'minecraft:%s_banner' % rng.choice(DYES)
            case['resultLayers'] = [[rng.choice(common), rng.choice(DYES)] for _ in range(rng.choice([1, 2, 6, 7]))]
        out.append(case)
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1600)
    parser.add_argument('--seed', type=int, default=3)
    parser.add_argument('--detail', type=int, default=2)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    work = lib.work_dir('screen-layout-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    chosen = cases(args.count, args.seed)
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    done = subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-cp', deps,
                           '-d', str(classes), *(str(lib.PARITY / f'{n}.java') for n in ('ParityData', 'VanillaLoomScreenOracle'))],
                          capture_output=True, text=True)
    if done.returncode:
        print(done.stderr[-4000:])
        sys.exit('javac failed')
    run = subprocess.run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + deps, 'dev.horizonlab.parity.VanillaLoomScreenOracle',
                          str(work / 'cases.jsonl'), str(work / 'native.jsonl')], capture_output=True, text=True)
    if run.returncode:
        print(run.stderr[-4000:])
        sys.exit('oracle failed')
    lib.cargo_example('horizon-client', 'screen_layout_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.jsonl')
    native = [json.loads(l) for l in (work / 'native.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (work / 'rust.jsonl').read_text().splitlines()]
    assert len(native) == len(rust) == len(chosen)
    bad, errors = {}, 0
    for case, a, b in zip(chosen, native, rust):
        if 'error' in a:
            errors += 1
            print('oracle error', a['error'])
            continue
        if a['draws'] != b['draws']:
            bad.setdefault(case['screen'], []).append((case, a['draws'], b['draws']))
    for screen, entries in bad.items():
        print(f'{screen}: {len(entries)} differing')
        for case, a, b in entries[:args.detail]:
            i = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            print('  case  ', json.dumps(case)[:400])
            print('  native', a[i] if i < len(a) else '-')
            print('  rust  ', b[i] if i < len(b) else '-')
    total = sum(len(v) for v in bad.values())
    print(f'{len(chosen) - total - errors}/{len(chosen)} screens match ({errors} oracle errors)')
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
