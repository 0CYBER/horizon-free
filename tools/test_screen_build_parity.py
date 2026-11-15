#!/usr/bin/env python3
"""Compare the container screens' built layout with the vanilla client's.

The Java oracle (VanillaScreenBuildOracle) runs the real `Window.calculateScale`
over each case's framebuffer, then the real `Screen.init(width, height)` of the
survival, crafting, furnace, chest (one to six rows), hopper and shulker
screens, with or without the recipe book's offset, and reports the panel
(`leftPos`/`topPos`/`imageWidth`/`imageHeight`), the slots' screen positions and
the slot `AbstractContainerScreen`'s hover test answers for the case's cursor
points. The game (`HORIZON_SCREEN_BUILD_FIXTURE`) builds the same screens'
`ScreenBuild::ContainerLayout` (and the pause screen's `PauseLayout`) and
reports the panel, the origin, the slot boxes, the slot `slot_at` answers and
the pause menu's buttons and icons. For chat in bed and the command block and
minecart edit screens, both sides open the screen at another size, resize it
to the case's and report every widget's rectangle (vanilla's `children()`).
Both sides must agree; the first
comparison is the GUI scale and size, so a size mismatch cannot pass.
"""
import argparse
import json
import os
import random
import subprocess
import sys
import tempfile

try:
    from . import lab, test_model_baking_parity as baking, test_rust_parity as base
    from .integration_env import run_game
except ImportError:
    import lab
    import test_model_baking_parity as baking
    import test_rust_parity as base
    from integration_env import run_game

WINDOWS = [[320, 240], [854, 480], [1280, 720], [1920, 1080], [2560, 1440], [3840, 2160], [1279, 719]]
SCALES = [1, 2, 3, 4, 0]
# kind -> (imageWidth, imageHeight); the chest's height follows its rows.
IMAGES = {'inventory': (176, 166), 'crafting': (176, 166), 'furnace': (176, 166),
          'hopper': (176, 133), 'shulker': (176, 167)}
BOOKS = ('inventory', 'crafting', 'furnace')
NARROW = 379
TEXT_SCREENS = ('chat', 'command_block', 'minecart_command_block')


def calculate_scale(window, max_scale):
    """`Window.calculateScale`: the largest scale keeping >= 320x240 GUI pixels."""
    width, height = window
    scale = 1
    while scale != max_scale and scale < width and scale < height and width // (scale + 1) >= 320 \
            and height // (scale + 1) >= 240:
        scale += 1
    return scale


def gui_size(window, max_scale):
    scale = calculate_scale(window, max_scale)
    return scale, -(-window[0] // scale), -(-window[1] // scale)


def image(kind, rows):
    return IMAGES.get(kind) or (176, 114 + 18 * rows)


def points(rng, left, top, width, height):
    """Cursor points over the panel, its slot grid, its edges and just outside."""
    out = []
    for gy in range(0, height, 14):
        for gx in range(0, width, 14):
            out.append([left + gx + rng.choice([0, 0.5, 1, 16, 17]),
                        top + gy + rng.choice([0, 0.5, 1, 16, 17])])
    for d in (-2, -1, 0, 1, 2):
        out.append([left + d, top + height / 2])
        out.append([left + width + d, top + height / 2])
        out.append([left + width / 2, top + d])
        out.append([left + width / 2, top + height + d])
    rng.shuffle(out)
    return out[:64]


def cases():
    kinds = sorted(IMAGES) + [f'chest{rows}' for rows in range(1, 7)]
    rng = random.Random(0x53435242)
    out = []
    for window in WINDOWS:
        for scale in SCALES:
            gui_scale, gui_width, gui_height = gui_size(window, scale)
            out.append({'kind': 'pause', 'window': window, 'scale': scale})
            for kind in TEXT_SCREENS:
                out.append({'kind': kind, 'window': window, 'scale': scale})
            for kind in kinds:
                rows = int(kind[5:]) if kind.startswith('chest') else 0
                width, height = image(kind, rows)
                books = (True, False) if kind in BOOKS else (False,)
                for book in books:
                    left = (gui_width - width) // 2 + (77 if book and gui_width >= NARROW else 0)
                    top = (gui_height - height) // 2
                    out.append({'kind': 'chest' if rows else kind, 'rows': rows or 3, 'book': book,
                                'window': window, 'scale': scale, 'points': points(rng, left, top, width, height)})
    return out


def run(command, **kwargs):
    done = subprocess.run(command, capture_output=True, text=True, **kwargs)
    if done.returncode:
        print(done.stdout[-2000:])
        print(done.stderr[-4000:])
        sys.exit(f'failed: {command[0]}')
    return done


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--detail', type=int, default=4)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'screen-build-parity'
    work.mkdir(parents=True, exist_ok=True)
    chosen = cases()
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    # A game that dies before writing its output must not pass on an old file.
    for stale in ('native.jsonl', 'rust.jsonl'):
        (work / stale).unlink(missing_ok=True)
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root),
         '-nowarn', '-cp', dependencies, '-d', str(classes),
         str(java_root / 'dev/horizonlab/parity/VanillaScreenBuildOracle.java')])
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.horizonlab.parity.VanillaScreenBuildOracle', str(work / 'cases.jsonl'), str(work / 'native.jsonl')])
    run(['cargo', 'build', '-q', '--profile', 'fast', '-p', 'horizon-game', '--bin', 'horizon-client'], cwd=lab.ROOT,
        env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    executable = lab.cargo_target() / ('fast/horizon-client.exe' if os.name == 'nt' else 'fast/horizon-client')
    with tempfile.TemporaryDirectory(prefix='game-', dir=work) as game_dir:
        done = run_game([str(executable), '--resources', str(resources), '--game-dir', game_dir], cwd=work,
                        env=lab.game_env(HORIZON_SCREEN_BUILD_FIXTURE=f'{work / "cases.jsonl"},{work / "rust.jsonl"}'))
        if done.returncode:
            sys.exit(f'horizon-client fixture failed with exit code {done.returncode}')
    native = [json.loads(line) for line in (work / 'native.jsonl').read_text().splitlines()]
    rust = [json.loads(line) for line in (work / 'rust.jsonl').read_text().splitlines()]
    assert len(native) == len(rust) == len(chosen), (len(native), len(rust), len(chosen))
    bad, errors = [], 0
    for case, a, b in zip(chosen, native, rust):
        if 'error' in a or 'error' in b:
            errors += 1
            print('error', case['kind'], a.get('error'), b.get('error'))
            continue
        # The first comparison: Window.calculateScale and its GUI size.
        if a['gui'] != b['gui'] or a['gui'][0] != calculate_scale(case['window'], case['scale']) \
                or a['gui'][1:] != list(gui_size(case['window'], case['scale'])[1:]):
            bad.append((case, 'gui', a['gui'], b['gui']))
            continue
        if case['kind'] == 'pause':
            if a['pause'] != b['pause']:
                bad.append((case, 'pause', a['pause'], b['pause']))
            continue
        if case['kind'] in TEXT_SCREENS:
            if a['widgets'] != b['widgets']:
                bad.append((case, 'widgets', a['widgets'], b['widgets']))
            continue
        for key in ('image', 'origin'):
            if a[key] != b[key]:
                bad.append((case, key, a[key], b[key]))
        if sorted(map(tuple, a['slots'])) != sorted(map(tuple, b['slots'])):
            bad.append((case, 'slots', a['slots'], b['slots']))
        if a['hits'] != b['hits']:
            differing = [i for i, (x, y) in enumerate(zip(a['hits'], b['hits'])) if x != y]
            bad.append((case, 'hits', [a['hits'][i] for i in differing],
                        [b['hits'][i] for i in differing]))
    for case, key, vanilla, horizon in bad[:args.detail]:
        labels = f'{case["kind"]}'
        if 'rows' in case:
            labels += f' rows {case["rows"]} book {case["book"]}'
        print(f'{labels} {case["window"]} scale {case["scale"]}: {key}')
        print('  vanilla', vanilla)
        print('  horizon', horizon)
    print(f'{len(chosen) - len(bad) - errors}/{len(chosen)} cases match ({errors} errors)')
    raise SystemExit(1 if bad or errors else 0)


if __name__ == '__main__':
    main()
