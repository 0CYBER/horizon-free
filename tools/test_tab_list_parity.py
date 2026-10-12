#!/usr/bin/env python3
"""Compare the player list (tab list) with the vanilla 26.3 client.

The Java oracle (VanillaTabListOracle) runs vanilla's own
PlayerTabOverlay.extractRenderState, frame after frame, on a stand-in client
whose connection lists the case's players (game mode, latency, display name,
list order, hat layer, online mode for heads), whose level holds some of
them (upside-down Dinnerbone and Grumm) and whose real Scoreboard has the
case's teams (colour, prefix, suffix), `list` objective (integer or hearts,
number formats) and scores, with the header, footer, row background option
and Hud ticks. It records every fill, text (as styled runs), sprite and
face blit. `examples/tab_list_fixture` lays out the same frames with
`horizon_client::tab_list`; the ordered call lists must agree.

  test_tab_list_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random
import subprocess
import sys
import uuid

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
COLORS = ['black', 'dark_blue', 'dark_green', 'dark_aqua', 'dark_red', 'dark_purple', 'gold', 'gray', 'dark_gray',
          'blue', 'green', 'aqua', 'red', 'light_purple', 'yellow', 'white']
WORDS = ['Welcome', 'to', 'the', 'server', 'Horizon', 'players', 'online', 'mc.example.net', 'Have', 'fun!',
         'Supercalifragilisticexpialidocious', 'a', 'é', 'Ünïcödé', '§aGreen', '§lBold', '[VIP]', '1234567890']
NAMES = ['Steve', 'Alex', 'Notch', 'jeb_', 'Dinnerbone', 'Grumm', 'xX_Pro_Xx', 'a', 'Zed', 'zebra', 'Émile', 'ÆON',
         'bob', 'Carl', '_under', '123', 'MMMMMMMMMMMMMMMM', 'iiiiiiii', 'Wide_W', 'ßtraße', 'Ⅻ', 'Mix3d_Case']


def component(rng, words=3, depth=0):
    text = ' '.join(rng.choice(WORDS) for _ in range(words))
    kind = rng.random()
    if kind < 0.3:
        return text
    c = {'text': text}
    if rng.random() < 0.5:
        c['color'] = rng.choice(COLORS + ['#12ABEF'])
    for flag in ('bold', 'italic', 'underlined', 'strikethrough', 'obfuscated'):
        if rng.random() < 0.15:
            c[flag] = rng.random() < 0.8
    if depth < 2 and rng.random() < 0.4:
        c['extra'] = [component(rng, rng.randint(1, 3), depth + 1) for _ in range(rng.randint(1, 2))]
    if rng.random() < 0.08:
        c = {'translate': 'multiplayer.player.list.hp', 'with': [text], 'color': rng.choice(COLORS)}
    return c


def number_format(rng):
    kind = rng.random()
    if kind < 0.4:
        return None
    if kind < 0.55:
        return {'type': 'blank'}
    if kind < 0.85:
        style = {'color': rng.choice(COLORS)}
        if rng.random() < 0.4:
            style['bold'] = True
        return {'type': 'styled', 'style': style}
    return {'type': 'fixed', 'value': component(rng, 1)}


def cases(count):
    rng = random.Random(0x544142)
    out = []
    for n in range(count):
        size = rng.choice([1, 1, 2, 3, 5, 8, 12, 19, 20, 21, 25, 40, 41, 60, 79, 80, 85, 100])
        names = []
        while len(names) < size:
            name = rng.choice(NAMES) if rng.random() < 0.5 else ''.join(
                rng.choice('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_') for _ in range(rng.randint(1, 16)))
            if name.lower() not in {x.lower() for x in names}:
                names.append(name)
        players = []
        for name in names:
            players.append({
                'uuid': str(uuid.UUID(int=rng.getrandbits(128))), 'name': name,
                'gameMode': rng.choice([0, 0, 0, 1, 2, 3]),
                'latency': rng.choice([-1, 0, 1, 149, 150, 299, 300, 599, 600, 999, 1000, 5000, rng.randint(0, 2000)]),
                'displayName': component(rng, rng.randint(1, 2)) if rng.random() < 0.2 else None,
                'listOrder': rng.choice([0, 0, 0, 1, -1, 5, 2147483647, -2147483648]),
                'showHat': rng.random() < 0.8, 'inLevel': rng.random() < 0.5,
            })
        teams = []
        for t in range(rng.choice([0, 0, 1, 2, 4])):
            members = [p['name'] for p in players if rng.random() < 0.3]
            teams.append({'name': rng.choice(['red', 'Blue', 'team' + str(t), 'ä', 'z', 'A']) + str(t),
                          'color': rng.choice([None] + COLORS),
                          'prefix': rng.choice(['', '[R] ', {'text': '★ ', 'color': 'gold', 'bold': True}, component(rng, 1)]),
                          'suffix': rng.choice(['', ' ✔', {'text': ' (afk)', 'italic': True}]), 'members': members})
        objective = None
        if rng.random() < 0.7:
            objective = {'name': 'obj', 'renderType': rng.choice(['integer', 'hearts', 'hearts']), 'numberFormat': number_format(rng)}
        frames = []
        tick = rng.randint(0, 100000)
        values = {}
        for f in range(rng.randint(1, 6)):
            scores = {}
            for p in players:
                if objective and (f == 0 and rng.random() < 0.85 or rng.random() < 0.3):
                    v = rng.choice([0, 1, 2, 7, 19, 20, 21, 40, 99, 100, -3, 1234567, rng.randint(-50, 300)])
                    if p['name'] in values and rng.random() < 0.5:
                        v = values[p['name']] + rng.choice([-3, -1, 1, 2, 10])
                    values[p['name']] = v
                    scores[p['name']] = {'value': v, 'format': number_format(rng) if rng.random() < 0.2 else None}
            reset = [p['name'] for p in players if objective and rng.random() < 0.03]
            for r in reset:
                values.pop(r, None)
            tick += rng.choice([1, 1, 2, 3, 5, 10, 21, 40])
            frames.append({'tick': tick, 'visible': rng.random() < 0.85, 'scores': scores, 'reset': reset})
        header = footer = None
        if rng.random() < 0.6:
            header = rng.choice([component(rng, rng.randint(1, 30)), 'Line one\nLine two', '\n', 'trailing\n', '  two  spaces  ',
                                 {'text': 'Header', 'extra': ['\n', component(rng, 12)]}])
        if rng.random() < 0.5:
            footer = rng.choice([component(rng, rng.randint(1, 30)), 'Footer', 'x\n\ny'])
        out.append({'width': rng.choice([427, 320, 480, 640, 961, 160, 90, 300]), 'onlineMode': rng.random() < 0.7,
                    'backgroundForChatOnly': rng.random() < 0.8, 'opacity': rng.choice([0.5, 0.0, 1.0, 0.37]),
                    'header': header, 'footer': footer, 'teams': teams, 'objective': objective, 'players': players,
                    'frames': frames})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1000)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-tab-list-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    chosen = cases(args.count)
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaTabListOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaTabListOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client', '--example', 'tab_list_fixture', '--',
         resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust)
    errors, bad, frames, calls = [], [], 0, 0
    for v, r in zip(vanilla, rust):
        if 'error' in v:
            errors.append(v['error']); continue
        for i, (a, b) in enumerate(zip(v['frames'], r['frames'])):
            frames += 1
            calls += len(a)
            if a != b:
                first = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                bad.append((v['case'], i, first, a[first:first + 2], b[first:first + 2], len(a), len(b)))
                break
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for case, i, k, a, b, la, lb in bad[:args.detail]:
        print('MISMATCH', json.dumps(case)[:3000])
        print(f'   frame {i}, call {k} ({la} vanilla, {lb} Horizon calls)\n   vanilla {a}\n   horizon {b}')
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} player lists match ({frames} frames, {calls} calls); '
          f'{len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('tab list parity ok')


if __name__ == '__main__':
    main()
