#!/usr/bin/env python3
"""Moving on wool makes no vibration on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's each serve a copy of one flat world. KnockbackClient, a player made of
vanilla's protocol code, sends the positions a real client sends for a jump, a drop and a walk, sneaking or not, on
stone (the control), wool, a wool carpet and a wool slab, and beside a hole at a block's edge; a walk runs with
ticking frozen, one position a stepped tick (`walk`). A cow, an item and an armor stand fall on the same floors. A sculk sensor stands three blocks away, and a shrieker beside it in two cases.

After each case the sensor's `last_vibration_frequency` and phase (and the shrieker's state) are read with commands.
A player hurt on each floor is the positive control: the same sensor hears it, so a silent sensor on wool is the
dampening (VibrationSystem.User.isValidVibration) and not an occluded signal.

Each case is compared on its own. KNOWN lists the cases Horizon still gets wrong and why; one that matches again is
reported, and should leave the list.
"""
import difflib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time

try:
    from . import fast_bins, lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, game_time, wait_ticks
    from .test_locator_bar_parity import ClientOutput, generate_world
    from .test_server_knockback_parity import SYNC_TIMEOUT, frozen, left, server_dir
except ImportError:
    import fast_bins
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, game_time, wait_ticks
    from test_locator_bar_parity import ClientOutput, generate_world
    from test_server_knockback_parity import SYNC_TIMEOUT, frozen, left, server_dir

NAME = 'server-wool-vibration-parity'
KNOWN = {}

SENSOR = (2, -60, 3)
SHRIEKER = (4, -60, 3)
START = (0.5, 0.5)
FAR = '0.5 -60 30.5'
FULL = -60.0
FLOORS = {
    'stone': (FULL, []),
    'wool': (FULL, ['fill -2 -61 -2 7 -61 1 minecraft:white_wool']),
    'carpet': (FULL + 0.0625, ['fill -2 -60 -2 7 -60 1 minecraft:white_carpet']),
    'slab': (FULL + 0.5, ['fill -2 -60 -2 7 -60 1 minecraft:white_wool_slab']),
    # The player's middle is over the hole: their supporting block is the one at x = 0.
    'stone-edge': (FULL, ['fill 1 -61 -2 7 -61 1 minecraft:air']),
    'wool-edge': (FULL, ['fill -2 -61 -2 0 -61 1 minecraft:white_wool', 'fill 1 -61 -2 7 -61 1 minecraft:air']),
}


def at(x, y, ground):
    return [f'pos Alice {x} {y} {START[1]} 0 0 {1 if ground else 0}', 'sleep 0.05']


def jump(x, ground):
    """LivingEntity.jumpFromGround's rise and the fall back, one client position a tick."""
    lines, y, v = [], ground, 0.41999998688697815
    while True:
        y += v
        v = (v - 0.08) * 0.98
        if y <= ground:
            return lines + at(x, ground, True) * 2
        lines += at(x, y, False)


def drop(x, ground, height=1.25):
    lines, y, v = [], ground + height, 0.0
    while True:
        v = (v - 0.08) * 0.98
        y += v
        if y <= ground:
            return lines + at(x, ground, True) * 2
        lines += at(x, y, False)


# Stepped ticks a walk is read over: its sensor (on stone) goes active at tick 11 after the first position, cooldown
# at 41 and inactive at 51 on both servers (#18133), so this covers the whole cycle and nine quiet ticks after it.
WALK_TICKS = 60


def walk(x, ground, case):
    """Twenty positions 0.2 blocks apart, one a game tick, with ticking frozen and stepped once a tick, and after every
    stepped tick the sensor's frequency and phase, filed under `case#tNN`: compared tick by tick, which pins when the
    sensor turns on, its 30 active and 10 cooldown ticks and its return to inactive. Real time read the phase once,
    a few ticks before the end of the active ticks (#18133). A stepped walk no longer covers when a server applies a
    position packet relative to its ticks: each position is confirmed applied (`moved`) before the tick runs."""
    lines = ['freeze']
    for tick in range(1, WALK_TICKS + 1):
        if tick <= 20:
            x_i = round(x + 0.2 * tick, 3)
            lines += [f'pos Alice {x_i} {ground} {START[1]} 0 0 1', f'moved {x_i} {ground}']
        lines += ['tickstep', f'step {case}#t{tick:02d}', FREQUENCY, PHASE, COOLDOWN]
    return lines + ['unfreeze']


def arena(floor):
    # The previous case's cow drops its loot as /kill runs, after the selector picked the cow, and its corpse stays
    # for 20 ticks (LivingEntity.tickDeath), still moving: the items, or the corpse once the fill below removes the
    # carpet or slab under it, hit the ground while the next case measures, at a wall-clock moment (item-stone,
    # item-carpet and item-slab flaked on vanilla's side, 2026-11-07). The second /kill removes the items, and 25
    # server ticks let the corpse go before the arena changes.
    return ['step setup', 'cmd Alice time set 1000', 'cmd Alice kill @e[type=!player]', 'cmd Alice kill @e[type=item]',
            'ticks 25',
            'cmd Alice fill -3 -61 -3 8 -61 5 minecraft:stone', 'cmd Alice fill -3 -60 -3 8 -57 5 minecraft:air',
            *[f'cmd Alice {command}' for command in FLOORS[floor][1]]]


def sensor(shrieker=False):
    lines = [f'cmd Alice setblock {SENSOR[0]} {SENSOR[1]} {SENSOR[2]} minecraft:sculk_sensor']
    if shrieker:
        lines.append(f'cmd Alice setblock {SHRIEKER[0]} {SHRIEKER[1]} {SHRIEKER[2]} minecraft:sculk_shrieker')
    return lines


FREQUENCY = f'cmd Alice data get block {SENSOR[0]} {SENSOR[1]} {SENSOR[2]} last_vibration_frequency'
PHASE = (f'cmd Alice execute if block {SENSOR[0]} {SENSOR[1]} {SENSOR[2]} '
         'minecraft:sculk_sensor[sculk_sensor_phase=active]')
COOLDOWN = (f'cmd Alice execute if block {SENSOR[0]} {SENSOR[1]} {SENSOR[2]} '
            'minecraft:sculk_sensor[sculk_sensor_phase=cooldown]')
SHRIEKING = (f'cmd Alice execute if block {SHRIEKER[0]} {SHRIEKER[1]} {SHRIEKER[2]} '
             'minecraft:sculk_shrieker[shrieking=true]')


def player_case(case, floor, action, sneak=False, shrieker=False):
    ground = FLOORS[floor][0]
    x = 1.05 if floor.endswith('-edge') else START[0]
    height = 1.25 if action is drop else 0
    lines = [*arena(floor), f'cmd Alice tp @s {x} {ground + height} {START[1]} 0 0',
             *at(x, ground + height, height == 0) * 3]
    if sneak:
        lines += ['sneak Alice on', 'sleep 0.1']
    lines += [*sensor(shrieker), 'ticks 5', f'step {case}', FREQUENCY]
    if action == 'hurt':
        lines.append('cmd Alice damage @s 1 minecraft:generic')
    elif action is walk:
        STEPPED.add(case)
        lines += walk(x, ground, case)
    else:
        lines += action(x, ground)
    if action is not walk:
        lines += ['ticks 12', FREQUENCY, PHASE]
    if shrieker:
        lines.append(SHRIEKING)
    lines += ['step setup', 'cmd Alice effect give @s minecraft:instant_health 1 5 true']
    if sneak:
        lines += ['sneak Alice off', 'sleep 0.1']
    return lines


def entity_case(case, floor, summon):
    ground = FLOORS[floor][0]
    x = 1.05 if floor.endswith('-edge') else START[0]
    return [*arena(floor), f'cmd Alice tp @s {FAR} 0 0', *['sleep 0.05'] * 3, *sensor(), 'ticks 5', f'step {case}', FREQUENCY,
            f'cmd Alice summon {summon.format(pos=f"{x} {ground + 3} {START[1]}")}', 'ticks 40', FREQUENCY,
            'step setup']


STEPPED = set()  # the cases player_case reads tick by tick
COW = 'minecraft:cow {pos} {{active_effects:[{{id:"minecraft:slowness",amplifier:127b,duration:1000}}]}}'
CASES = {}
for floor_name in ('stone', 'wool', 'carpet', 'slab'):
    CASES[f'hurt-{floor_name}'] = player_case(f'hurt-{floor_name}', floor_name, 'hurt')
    for action_name, action_fn in (('jump', jump), ('drop', drop), ('walk', walk)):
        CASES[f'{action_name}-{floor_name}'] = player_case(f'{action_name}-{floor_name}', floor_name, action_fn)
    for entity, summon in (('cow', COW),
                           ('item', 'minecraft:item {pos} {{Item:{{id:"minecraft:stone",count:1}},PickupDelay:32767s}}'),
                           ('stand', 'minecraft:armor_stand {pos}')):
        CASES[f'{entity}-{floor_name}'] = entity_case(f'{entity}-{floor_name}', floor_name, summon)
for floor_name in ('stone', 'wool'):
    CASES[f'sneak-jump-{floor_name}'] = player_case(f'sneak-jump-{floor_name}', floor_name, jump, sneak=True)
    CASES[f'sneak-walk-{floor_name}'] = player_case(f'sneak-walk-{floor_name}', floor_name, walk, sneak=True)
    CASES[f'shrieker-jump-{floor_name}'] = player_case(f'shrieker-jump-{floor_name}', floor_name, jump, shrieker=True)
for floor_name in ('stone-edge', 'wool-edge'):
    CASES[f'jump-{floor_name}'] = player_case(f'jump-{floor_name}', floor_name, jump)
    CASES[f'cow-{floor_name}'] = entity_case(f'cow-{floor_name}', floor_name, COW)


# Entity.kill emits ENTITY_DIE after removal; wool stacks dampen it at the
# source (ItemEntity.dampensVibrations), regardless of the floor material.
for name, kind, nbt in (
        ('stone-item', 'item', '{Item:{id:"minecraft:stone",count:1},NoGravity:1b}'),
        ('wool-item', 'item', '{Item:{id:"minecraft:white_wool",count:1},NoGravity:1b}'),
        ('red-wool-item', 'item', '{Item:{id:"minecraft:red_wool",count:1},NoGravity:1b}'),
        ('cushion', 'cushion', '{}'),
        ('crystal', 'end_crystal', '{Invulnerable:1b}'),
        ('marker', 'marker', '{}')):
    case = f'kill-{name}'
    CASES[case] = [*arena('stone'), f'cmd Alice tp @s {FAR} 0 0',
                   f'cmd Alice summon minecraft:{kind} 0.5 -60 0.5 {nbt}', 'ticks 10',
                   *sensor(), 'ticks 5', f'step {case}', FREQUENCY,
                   f'cmd Alice kill @e[type=minecraft:{kind}]', 'ticks 12', FREQUENCY, PHASE,
                   'step setup']


def position(server, timeout=2.0):
    """Alice's position, asked from the console, or None if no reply came in `timeout` s: the caller asks again
    (Horizon's server left one unanswered within 120 s on a hosted run, 11-20)."""
    start = len(server.lines)
    server.console('data get entity Alice Pos')
    deadline = time.time() + timeout
    while time.time() < deadline:
        for line in server.lines[start:]:
            if found := re.search(r'entity data: \[(.*)\]', line):
                return [float(v.strip().rstrip('d')) for v in found.group(1).split(',')]
        time.sleep(0.02)
    return None


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = server_dir(directory, world)
    out = build / f'{label}.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), str(out)], cwd=lab.ROOT,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            output = ClientOutput(client, log, f'{label} (see {build}/{label}-client.log)')

            def sync():
                client.stdin.write('sync Alice\n')
                client.stdin.flush()
                output.expect('synced Alice', SYNC_TIMEOUT)

            clock = [None]  # the frozen game clock while a walk is stepped

            def send(line):
                word = line.split()[0]
                if word == 'sleep':
                    client.stdin.flush()
                    time.sleep(float(line.split()[1]))
                elif word == 'ticks':
                    sync()
                    wait_ticks(server, int(line.split()[1]))
                    sync()
                elif word == 'freeze':
                    sync()
                    server.console('tick freeze')
                    clock[0] = frozen(server)
                elif word == 'unfreeze':
                    # After the reads before it have run: `cmd` lines are only queued on the client.
                    sync()
                    server.console('tick unfreeze')
                    sync()
                    clock[0] = None
                elif word == 'moved':
                    # A frozen server may apply a position in the player's next tick, not when it arrives.
                    sync()
                    x, y = map(float, line.split()[1:3])
                    deadline = time.time() + 30
                    failure = f'{label}: Alice was not at x {x} y {y} 30 s after moving'
                    while (pos := position(server, min(2.0, left(deadline, failure)))) is None \
                            or abs(pos[0] - x) > 1e-6 or abs(pos[1] - y) > 1e-6:
                        left(deadline, f'{failure} (at {pos})')
                        time.sleep(0.02)
                elif word == 'tickstep':
                    # One `/tick step 1` from the frozen clock: it answers at once and the tick runs after, and
                    # the clock must then read exactly one more, not stand still before it or run past it.
                    sync()
                    deadline = time.time() + 60
                    failure = f'{label}: /tick step 1 from game time {clock[0]} did not finish in 60 s'
                    if (now := game_time(server, left(deadline, failure))) != clock[0]:
                        raise AssertionError(f'{label}: the frozen clock moved from {clock[0]} to {now}')
                    server.console('tick step 1')
                    while (now := game_time(server, left(deadline, failure))) == clock[0]:
                        time.sleep(0.02)
                    if now != clock[0] + 1:
                        raise AssertionError(f'{label}: /tick step 1 moved the clock from {clock[0]} to {now}')
                    clock[0] = now
                    sync()
                else:
                    client.stdin.write(line + '\n')
                    client.stdin.flush()
            try:
                send('join Alice')
                server.wait_for(r'Alice joined the game', timeout=120)
                wait_ticks(server, 40)
                for lines in CASES.values():
                    for line in lines:
                        send(line)
                send('end')
                client.stdin.close()
                output.drain(120)
                client.wait(timeout=10)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    return out.read_text(encoding='utf-8').splitlines()


def row(tick, texts):
    """One stepped tick's reads (FREQUENCY, PHASE, COOLDOWN) as (`tNN frequency F phase`, None), or (the replies as
    they came, why they are no observation): a missing or extra reply, an error or timeout, a reply that is not
    the expected one, or both phase predicates true."""
    raw = f'{tick} ' + ' | '.join(texts)
    if len(texts) != 3:
        return raw, f'{len(texts)} replies, not 3: {texts}'
    found = re.fullmatch(r'chat .* has the following block data: (-?\d+)', texts[0])
    tests = [text.removeprefix('chat ') for text in texts[1:]]
    if not found or any(test not in ('Test passed', 'Test failed') for test in tests):
        return raw, f'unreadable replies {texts}'
    active, cooldown = (test == 'Test passed' for test in tests)
    if active and cooldown:
        return raw, 'both the active and the cooldown phase test passed'
    return f'{tick} frequency {found.group(1)} {"active" if active else "cooldown" if cooldown else "inactive"}', None


def stepped_problems(cases, problems):
    """Why each stepped case's rows are no complete set of observations: `problems` (from `compared`) plus a row
    count other than WALK_TICKS. Empty when every one of the WALK_TICKS ticks gave a valid row."""
    out = {}
    for case in STEPPED:
        rows = [r for r in cases.get(case, []) if r[:1] == 't' and r[1:3].isdigit()]
        found = list(problems.get(case, []))
        if len(rows) != WALK_TICKS:
            seen = {r.split(' ', 1)[0] for r in rows}
            missing = [f't{n:02d}' for n in range(1, WALK_TICKS + 1) if f't{n:02d}' not in seen]
            gap = f' (missing {", ".join(missing[:5])}{" ..." if len(missing) > 5 else ""})' if missing else ''
            found.insert(0, f'{len(rows)} stepped rows, not {WALK_TICKS}{gap}')
        if found:
            out[case] = found
    return out


def compared(lines, problems=None):
    """Per case, the lines compared. A stepped case's ticks become one row each; `problems` gets, per case, why a
    tick's row is no observation (`row`)."""
    cases, ticks = {}, {}
    for line in lines:
        _, step, text = line.split(' ', 2)
        case, _, tick = step.removeprefix('step=').partition('#')
        # `[Server: ...]` is the server echoing the console's `tick` commands to an operator: not under test.
        if case in CASES and text.startswith(('chat ', 'teleport ', 'error', 'disconnected')) \
                and not text.startswith('chat [Server: '):
            if tick:
                ticks.setdefault(case, {}).setdefault(tick, []).append(text)
            else:
                cases.setdefault(case, []).append(text)
    for case, rows in ticks.items():
        for tick, texts in rows.items():
            text, problem = row(tick, texts)
            cases.setdefault(case, []).append(text)
            if problem and problems is not None:
                problems.setdefault(case, []).append(f'{tick}: {problem}')
    return cases


def changes(rows):
    """A stepped case's rows where something changed: its whole sequence in a few entries."""
    return [r for i, r in enumerate(rows) if i == 0 or r.split(' ', 1)[-1] != rows[i - 1].split(' ', 1)[-1]]


def first_difference(want, got):
    """The first differing row of two stepped sequences and the rows beside it, on each side."""
    i = next((i for i, (a, b) in enumerate(zip(want, got)) if a != b), min(len(want), len(got)))
    return f'first difference at row {i}: vanilla {want[max(0, i - 1):i + 2]} horizon {got[max(0, i - 1):i + 2]}'


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/KnockbackClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    subprocess.run(fast_bins.command('horizon-client'),
                   cwd=lab.ROOT, check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.KnockbackClient']

    world = generate_world(vanilla_server, build)
    vanilla_problems, horizon_problems = {}, {}
    vanilla = compared(run_side('vanilla', vanilla_server, client, build, world), vanilla_problems)
    horizon = compared(run_side('horizon', horizon_server, client, build, world), horizon_problems)
    unusable = {side: stepped_problems(cases, problems)
                for side, cases, problems in (('vanilla', vanilla, vanilla_problems),
                                              ('horizon', horizon, horizon_problems))}
    differ, fixed = [], []
    for case in CASES:
        want, got = vanilla.get(case, []), horizon.get(case, [])
        bad = [f'{side} {problem}' for side in unusable for problem in unusable[side].get(case, [])]
        if bad:
            # Not compared: a row that is no observation on both sides would compare equal and hide a difference.
            differ.append(f'{case}: the stepped reads are incomplete: ' + '; '.join(bad[:4]))
            continue
        shown, other = (changes(want), changes(got)) if case in STEPPED else (want, got)
        print(f'{"same  " if want == got else "DIFFER"} {case}: vanilla {shown}' + ('' if want == got else f' horizon {other}'))
        if not want or any(line.startswith(('error', 'disconnected')) for line in want):
            raise AssertionError(f'{case}: vanilla recorded nothing or failed: {want}')
        if want == got:
            if case in KNOWN:
                fixed.append(case)
        elif case in STEPPED and case not in KNOWN:
            differ.append(f'{case}: {first_difference(want, got)}')
        elif case not in KNOWN:
            differ.append(f'{case}:\n' + '\n'.join(difflib.unified_diff(want, got, 'vanilla', 'horizon', n=1, lineterm='')))
    if fixed:
        print('matching vanilla now, take them out of KNOWN: ' + ', '.join(fixed))
    if differ:
        raise AssertionError(f'{len(differ)} cases differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(differ)[:8000])
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        cases=len(CASES), known=sorted(KNOWN), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='A sculk sensor (and a shrieker) on a dedicated server, as a vanilla-codec player jumps, drops, walks '
              'and sneaks on stone, wool, a wool carpet and a wool slab or at a block\'s edge, and as a cow, an item '
              'and an armor stand fall on the same floors.'))
    print(f'server wool vibration: {len(CASES) - len(KNOWN)} of {len(CASES)} cases identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
