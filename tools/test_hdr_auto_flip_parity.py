#!/usr/bin/env python3
"""HDR Auto follows the display without meshing the world again (#12989): Auto on -> off -> on.

One game runs offscreen on the float frame (HORIZON_HDR_FRAME=float, HORIZON_HDR_OUTPUT=pq) with HDR on Auto, under
HORIZON_STILL=1. No HDR display is needed: HORIZON_HDR_DISPLAY names a file that says what the system reports for the
display, `on` or `off`, which the game reads every two seconds as it reads the system. The file starts at `on`, turns
`off`, then `on` again, and the scenes are shot in each state. A second game starts with the file at `off`. A third
flips as the first under the shader API 0.2 example pack (golden_hour_0_2) with flat draws on (HORIZON_FLAT_DRAWS=1),
where a Horizon pack lays solid terrain out wide instead of compact (`packed_layout`).

It fails unless:
- HDR output follows the file: the shots have their -hdr.png while it says `on`, and none while it says `off`;
- the renderer was made again for each frame (the 8-bit one, then the float one), so the flips were real;
- no section was meshed again by either flip (`meshed_sections` of horizon-ctl status), and as many stay resident;
- the pictures after on -> off -> on are the first ones, pixel for pixel, the -hdr.png too;
- the pictures while `off` are those of the game that started at `off`;
- under the pack, the same, but against no game started at `off`; and the pack ran: solid terrain is laid out wide,
  and its pictures differ from the first game's.

Horizon's own feature: there is no vanilla oracle, and the pictures compared are all this build's.

  test_hdr_auto_flip_parity.py [--bin-dir DIR] [--resources DIR] [-j N]
"""
import argparse
import json
import os
import shutil
import sys
import time
from pathlib import Path

try:
    from . import lab
    from . import test_frame_identity_parity as identity
except ImportError:
    import lab
    import test_frame_identity_parity as identity

RUN = lab.LOCAL / 'hdr-auto-flip'
SCENES = ['day_water', 'night_lights']
ENV = {'HORIZON_HDR_FRAME': 'float', 'HORIZON_HDR_OUTPUT': 'pq'}
MADE_AGAIN = 'the renderer was made again for the frame'
PACK = 'golden_hour_0_2'


def status(game):
    reply = game.ctl('status')
    return reply['meshed_sections'], reply['renderer']['resident_sections']


def made_again(game):
    return str(game.ctl('logs', '--lines', '4000', check=False)).count(MADE_AGAIN)


def flip(game, display, to):
    """The system's HDR turns `to`: once the game has made its renderer for the other frame (the display is read
    every two seconds, and Auto waits half a second more before it turns off), how many sections that meshed and
    how many are resident before and after."""
    game.settle()
    before, made = status(game), made_again(game)
    display.write_text(to)
    deadline = time.monotonic() + 60
    while made_again(game) == made:
        if time.monotonic() > deadline:
            raise RuntimeError(f'the renderer did not follow the display turning {to}')
        game.ctl('wait', 5)
    game.settle()
    return before, status(game)


def session(name, bin_dir, out, env_base, args, flips, pack=None):
    """A game that starts with the display at `flips[0]` and follows the rest, the scenes shot in each state as
    <name>.<n>.<scene>.png, under `pack` if given. The (before, after) counts of each flip."""
    folder = RUN / 'games' / name
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    (folder / 'horizon-graphics.txt').write_text(identity.GRAPHICS + 'hdr:auto\n')
    (folder / 'options.txt').write_text('chatVisibility:2\ntutorialStep:none\npauseOnLostFocus:false\n')
    display = folder / 'display.txt'
    display.write_text(flips[0])
    extra = {}
    if pack:
        shutil.copytree(lab.ROOT / 'crates/horizon-shader-api/packs' / pack, folder / 'shaderpacks' / pack)
        extra = {'HORIZON_SHADER_PACK': pack, 'HORIZON_FLAT_DRAWS': '1'}
    env = lab.game_env(env_base, HORIZON_STILL='1', HORIZON_HDR_DISPLAY=display, **ENV, **extra)
    game = identity.Game(bin_dir, folder, env, args)
    counts = []
    try:
        game.launch()
        identity.prepare(game)
        for n, to in enumerate(flips):
            if n:
                counts.append(flip(game, display, to))
            for scene in SCENES:
                identity.shoot_named(game, out, f'{name}.{n}', scene)
        lab.write_json(out / f'{name}.layout.json', {'packed_layout': game.ctl('status')['renderer']['packed_layout'],
                                                     'flat': game.ctl('section-draws')['flat']})
        (out / f'{name}.log').write_text(str(game.ctl('logs', '--lines', '4000', check=False)), encoding='utf-8')
    finally:
        game.quit()
    return counts


def same(out, a, b, suffix):
    problem = identity.compare(out / f'{a}{suffix}', out / f'{b}{suffix}', out / f'diff-{a}{suffix}')
    return [] if problem is None else [f'{a}{suffix} against {b}{suffix}: {problem}']


def checks(out, name, counts):
    problems = []
    for n, on in enumerate((True, False, True)):
        for scene in SCENES:
            if not (out / f'{name}.{n}.{scene}.png').is_file():
                problems.append(f'{name}.{n}.{scene}: screenshot missing')
            if (out / f'{name}.{n}.{scene}-hdr.png').exists() != on:
                problems.append(f'{name}.{n}.{scene}: HDR output was {"off" if on else "on"}')
    for to, ((meshed, resident), (meshed_after, resident_after)) in zip(('off', 'on'), counts):
        if meshed_after != meshed:
            problems.append(f'{name}: the display turning {to} meshed {meshed_after - meshed} sections again')
        if resident_after != resident:
            problems.append(f'{name}: the display turning {to}: {resident_after} sections resident, {resident} before')
    for scene in SCENES:
        if problems:
            break
        problems += same(out, f'{name}.2.{scene}', f'{name}.0.{scene}', '.png')
        problems += same(out, f'{name}.2.{scene}', f'{name}.0.{scene}', '-hdr.png')
        if name == 'flip':
            problems += same(out, f'{name}.1.{scene}', f'off.0.{scene}', '.png')
    return problems


def pack_ran(out):
    """That the pack drew: solid terrain laid out wide, as for a Horizon pack, and pictures unlike the first game's."""
    problems = []
    layout = json.loads((out / 'pack.layout.json').read_text())
    # Without flat draws (lavapipe) solid terrain is wide with or without a pack, so only the pictures tell.
    if not layout['flat']:
        print('pack: no flat draws on this device: the wide layout is not checked, only the pictures')
    elif layout['packed_layout'][0]:
        problems.append('pack: solid terrain stayed compact: no Horizon pack ran')
    for n in range(3):
        for scene in SCENES:
            for suffix in ('.png', '-hdr.png'):
                shots = out / f'flip.{n}.{scene}{suffix}', out / f'pack.{n}.{scene}{suffix}'
                if all(s.exists() for s in shots) and identity.compare(*shots, None) is None:
                    problems.append(f'pack.{n}.{scene}{suffix}: the same as without the pack: it did not run')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build them)')
    parser.add_argument('--resources', default=str(lab.RESOURCES), help='the prepared resources the game reads')
    parser.add_argument('-j', '--jobs', type=int, help='cargo build jobs')
    args = parser.parse_args()
    bin_dir = Path(args.bin_dir).resolve() if args.bin_dir else identity.build(lab.ROOT, lab.cargo_target(), args.jobs)
    env = dict(os.environ)
    icd = identity.lavapipe()
    if icd:
        env.update(VK_DRIVER_FILES=str(icd), VK_ICD_FILENAMES=str(icd), WGPU_BACKEND='vulkan')
    out = RUN / 'shots'
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    counts = session('flip', bin_dir, out, env, args, ('on', 'off', 'on'))
    session('off', bin_dir, out, env, args, ('off',))
    pack_counts = session('pack', bin_dir, out, env, args, ('on', 'off', 'on'), PACK)
    print(f'meshed and resident sections, before and after each flip: {counts}', flush=True)
    print(f'the same under {PACK}: {pack_counts}', flush=True)
    problems = checks(out, 'flip', counts) + checks(out, 'pack', pack_counts) + pack_ran(out)
    lab.write_json(RUN / 'report.json', {'counts': counts, 'pack_counts': pack_counts, 'failures': problems})
    print('hdr auto flip:', 'PASS' if not problems else f'{len(problems)} FAILED')
    for problem in problems:
        print('  ', problem)
    sys.exit(1 if problems else 0)


if __name__ == '__main__':
    main()
