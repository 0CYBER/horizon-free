#!/usr/bin/env python3
"""LAN multiplayer end to end: a host opens its world to LAN and a guest joins, both real games.

Two Horizon clients run side by side under horizon-ctl, each in its own scratch folder. The host
creates a survival world and opens it to LAN with commands allowed; the guest connects to it by
address. The script then checks, from both sides, what a guest's game used to get wrong (#366):

- tab:       the guest's player list holds the host and the guest, under their own names;
- self:      the guest's own model exists (third person, inventory preview);
- armour:    each side draws the other player in that player's own armour;
- nametags:  each side draws the other player's name above them;
- chest:     a chest the guest opens opens on the guest's screen, its lid opens on both sides and
             closes on both when the guest closes it;
- crafting:  the guest crafts planks in the inventory's 2x2 grid and opens a crafting table;
- damage:    a guest in survival takes fall damage;
- mobs:      with the host in creative, a zombie and a skeleton go for the guest and hurt them;
- hazards:   a guest in survival gets hungry sprint-jumping and is hurt by lava, a block at the eyes,
             drowning and the void (then respawns);
- pvp:       the guest and the host hit each other with a sword; with the pvp game rule off, nothing;
- containers: the guest's ender chest holds the guest's items, not the host's, and a double chest
             opens as 9x6 with the right half first;
- feedback:  the guest's command feedback names the guest and shows the numbers.
- push:      a cow standing in the guest pushes it just clear of the cow (the guest's own client
             pushes it, as in vanilla), not ever faster across the ground.

Each check is reported on its own (a failing one doesn't stop the others) and the screenshots of
both sides go to --out (host-*.png, guest-*.png).

    python3 tools/lan_e2e.py                          # build with the fast profile, run every check
    python3 tools/lan_e2e.py --bin-dir target/fast    # prebuilt binaries
    python3 tools/lan_e2e.py --check chest --check tab

Needs the prepared resources in .local/horizon-client and, on Linux, vscreen (the desktop); on
macOS both games render offscreen. Games started here have no audio (HORIZON_NO_AUDIO=1).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_control_e2e import Cli, Failed, game_env  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
GUEST_NAME = 'Guest'
CHECKS = ['tab', 'self', 'armour', 'nametags', 'walls', 'chest', 'crafting', 'damage', 'mobs', 'hazards', 'pvp', 'containers', 'feedback', 'trials', 'push']


class Game:
    """One client: its scratch folder, its socket and a CLI driver."""

    def __init__(self, name, ctl, resources):
        self.name = name
        self.work = Path(tempfile.mkdtemp(prefix=f'hzlan-{name}-'))
        (self.work / '.local/horizon-client').mkdir(parents=True)
        os.symlink(resources, self.work / '.local/horizon-client/26.3')
        self.socket = self.work / 'c.sock'
        env = game_env(os.environ.get('VSCREEN', 'lan-e2e'))
        env['HORIZON_HOME'] = str(self.work / 'home')
        self.d = Cli(ctl, self.socket, self.work, env)
        self.d.name = name

    def __call__(self, tool, **args):
        return self.d(tool, **args)

    def raw(self, *words):
        """A game command horizon-ctl has no table entry for (lan, connect): the words as typed."""
        result = subprocess.run(self.d.base + [str(w) for w in words], cwd=self.work, env=self.d.env,
                                capture_output=True, text=True, timeout=600)
        lines = result.stdout.strip().splitlines()
        reply = json.loads(lines[-1]) if lines else {'error': result.stderr.strip()}
        print(f'[{self.name}] {list(words)} -> {json.dumps(reply)[:400]}')
        if result.returncode or 'error' in reply:
            raise Failed(f'{" ".join(map(str, words))}: {reply}')
        return reply

    def shot(self, out, label):
        path = out / f'{self.name}-{label}.png'
        self.d('screenshot', path=path)
        return path

    def wait(self, ticks):
        self.d('wait', ticks=ticks)

    def close(self):
        try:
            self.d('quit')
        except Exception:
            pass
        time.sleep(2)
        log = self.socket.with_suffix('.log')
        return log.read_text(errors='replace') if log.exists() else ''


def remote(guest):
    """The guest's view of the server (`status`'s server part)."""
    return guest('status')['server']


def back_to_game(game):
    """Close whatever screen a failed check left open, without opening the pause menu."""
    try:
        if game('state').get('screen') not in (None, 'playing'):
            game.raw('key', 'esc', 'tap')
            game.wait(2)
    except Failed:
        pass


def pos_of(game):
    return game('state')['player']['position']


def face_each_other(host, guest):
    """Put the guest 3 blocks east of the host, both looking at the other's head."""
    hx, hy, hz = pos_of(host)
    guest('command', command=f'tp @s {hx + 3:.2f} {hy:.2f} {hz:.2f}')
    guest.wait(30)
    host.raw('look-at', f'{hx + 3:.2f}', f'{hy + 1.6:.2f}', f'{hz:.2f}')
    guest.raw('look-at', f'{hx:.2f}', f'{hy + 1.6:.2f}', f'{hz:.2f}')
    host.wait(10)


def check_tab(host, guest, out):
    players = remote(guest)['players']
    names = sorted(p['name'] if isinstance(p, dict) else str(p) for p in players)
    assert names == sorted([host.player, GUEST_NAME]), f'guest tab list: {players}, host {host.player}'
    guest.raw('key', 'tab', 'down')
    guest.wait(5)
    guest.shot(out, 'tab')
    guest.raw('key', 'tab', 'up')
    return names


def check_self(host, guest, out):
    guest.raw('key', 'f5', 'tap')
    guest.wait(10)
    guest.shot(out, 'self-third-person')
    status = guest('state')
    guest.raw('key', 'f5', 'tap')
    guest.raw('key', 'f5', 'tap')
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    guest.shot(out, 'self-inventory')
    model = remote(guest).get('player_model')
    guest.raw('key', 'esc', 'tap')
    assert model, f'the guest has no player model: {status.get("camera")}'
    return 'model drawn'


def check_armour(host, guest, out):
    host('command', command='item replace entity @s armor.head with minecraft:golden_helmet')
    host('command', command='item replace entity @s armor.chest with minecraft:golden_chestplate')
    guest('command', command='item replace entity @s armor.chest with minecraft:diamond_chestplate')
    guest('command', command='item replace entity @s armor.legs with minecraft:diamond_leggings')
    face_each_other(host, guest)
    host.shot(out, 'armour')
    guest.shot(out, 'armour')
    seen = host.raw('lan', 'status')['guests'][0].get('model_armor')
    assert seen == [None, 'minecraft:diamond_chestplate', 'minecraft:diamond_leggings', None], \
        f'the host draws the guest in {seen}'
    host_gear = [e for e in remote(guest).get('player_equipment', [])]
    assert any('golden_chestplate' in json.dumps(e) for e in host_gear), f'the guest sees the host in {host_gear}'
    return 'each in their own'


def check_nametags(host, guest, out):
    face_each_other(host, guest)
    h = host('status').get('name_tags') or []
    g = guest('status').get('name_tags') or []
    host.shot(out, 'nametags')
    guest.shot(out, 'nametags')
    assert GUEST_NAME in h, f'host draws name tags {h}'
    assert host.player in g, f'guest draws name tags {g}'
    return {'host': h, 'guest': g}


def lid(game, pos):
    return game.raw('block', *pos).get('lid', 0.0)


def check_chest(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest = (x, y, z + 2)
    guest('command', command=f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest')
    guest.wait(10)
    guest.raw('interact', *chest)
    screen = guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(20)
    opened = {'guest': lid(guest, chest), 'host': lid(host, chest)}
    host.raw('look-at', chest[0] + 0.5, chest[1] + 0.5, chest[2] + 0.5)
    host.wait(2)
    host.shot(out, 'chest-open')
    guest.shot(out, 'chest-open')
    guest.raw('key', 'esc', 'tap')
    guest.wait(20)
    closed = {'guest': lid(guest, chest), 'host': lid(host, chest)}
    host.shot(out, 'chest-closed')
    assert screen, 'no container screen on the guest'
    assert opened['guest'] > 0.9 and opened['host'] > 0.9, f'lid while open: {opened}'
    assert closed['guest'] < 0.1 and closed['host'] < 0.1, f'lid after closing: {closed}'
    return {'open': opened, 'closed': closed}


def check_crafting(host, guest, out):
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_log 1')
    guest.wait(10)
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    inv = guest('inventory')
    slot = next(s['slot'] for s in inv['hotbar'] if s and s.get('item') == 'minecraft:oak_log')
    # Horizon's slot ids (click-slot): the hotbar is 0-8, the grid 100-108, the result 200.
    guest.raw('click-slot', slot)
    guest.raw('click-slot', 100)
    guest.wait(5)
    result = [s for s in guest('screen').get('slots', []) if s.get('slot') == 200]
    guest.shot(out, 'crafting-grid')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(5)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    planks = guest('inventory')['counts'].get('minecraft:oak_planks', 0)
    assert planks == 4, f'crafted {planks} planks (result slot: {result})'
    x, y, z = (int(v // 1) for v in pos_of(guest))
    table = (x, y, z - 2)
    guest('command', command=f'setblock {table[0]} {table[1]} {table[2]} minecraft:crafting_table')
    guest.wait(10)
    guest.raw('interact', *table)
    guest.raw('wait-for', 'screen', 'inventory', '--timeout', 10)
    size = guest('screen').get('craft_size')
    assert size == 3, f'the crafting table opened a {size}x{size} grid'
    # Two planks one above the other in the 3x3 grid: four sticks.
    slot = next(s['slot'] for s in guest('inventory')['hotbar'] if s and s.get('item') == 'minecraft:oak_planks')
    guest.raw('click-slot', slot)
    guest.raw('click-slot', 100, 'right')
    guest.raw('click-slot', 103, 'right')
    guest.raw('click-slot', slot)
    guest.wait(5)
    guest.shot(out, 'crafting-table')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(5)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    counts = guest('inventory')['counts']
    assert counts.get('minecraft:stick') == 4 and counts.get('minecraft:oak_planks') == 2, f'after the table: {counts}'
    return {'planks': planks, 'table': counts}


def check_damage(host, guest, out):
    guest('command', command='gamemode survival')
    guest.wait(10)
    before = remote(guest)['health']
    x, y, z = pos_of(guest)
    guest('command', command=f'tp @s {x:.2f} {y + 12:.2f} {z:.2f}')
    guest.wait(50)
    after = remote(guest)['health']
    guest.shot(out, 'damage')
    assert after < before, f'health {before} -> {after} after a 12-block fall'
    # Into water: the fall does no damage.
    x, y, z = pos_of(guest)
    guest('command', command=f'setblock {int(x // 1)} {int(y // 1)} {int(z // 1)} minecraft:water')
    guest('command', command=f'tp @s {x:.2f} {y + 12:.2f} {z:.2f}')
    guest.wait(50)
    water = remote(guest)['health']
    assert water >= after, f'health {after} -> {water} after falling into water'
    return {'before': before, 'after': after, 'water': water}


def first_hurt(guest, before, ticks):
    """The guest's health once it drops below `before`, waiting at most `ticks`."""
    health = remote(guest)['health']
    for _ in range(ticks // 10):
        if health < before:
            break
        guest.wait(10)
        health = remote(guest)['health']
    return health


def check_mobs(host, guest, out):
    host('command', command='gamemode creative')
    host('command', command='difficulty normal')
    host('command', command='time set midnight')
    guest('command', command='gamemode survival')
    guest.wait(10)
    hurt = {}
    try:
        for mob, distance in (('zombie', 3), ('skeleton', 7)):
            x, y, z = pos_of(guest)
            before = remote(guest)['health']
            host('command', command=f'summon minecraft:{mob} {x:.2f} {y:.2f} {z + distance:.2f}')
            low = first_hurt(guest, before, 300)
            guest.shot(out, f'mobs-{mob}')
            host.shot(out, f'mobs-{mob}')
            host('command', command=f'kill @e[type=minecraft:{mob}]')
            hurt[mob] = (before, low)
    finally:
        host('command', command='gamemode survival')
        host('command', command='time set day')
    for mob, (before, low) in hurt.items():
        assert low < before, f'the {mob} never hurt the guest: health {before} -> {low} (all: {hurt})'
    return hurt


def check_hazards(host, guest, out):
    guest('command', command='gamemode survival')
    # A fresh life: full health, so no regeneration eats food while the guest runs.
    guest('command', command='kill @s')
    guest.wait(20)
    guest.raw('respawn')
    guest.wait(40)
    # Sprint-jumping down a stone lane: ServerPlayer.jumpFromGround and checkMovementStatistics.
    guest('command', command='fill ~ ~-1 ~-1 ~60 ~-1 ~1 minecraft:stone')
    guest('command', command='fill ~ ~ ~-1 ~60 ~3 ~1 minecraft:air')
    guest.wait(10)
    x, y, z = pos_of(guest)
    guest.raw('look-at', f'{x + 60:.2f}', f'{y + 1.6:.2f}', f'{z:.2f}')
    food = host.raw('lan', 'status')['guests'][0]['food']
    for k in ('w', 'ctrl', 'space'):
        guest.raw('key', k, 'down')
    guest.wait(160)
    for k in ('space', 'ctrl', 'w'):
        guest.raw('key', k, 'up')
    guest.wait(10)
    fed = host.raw('lan', 'status')['guests'][0]['food']
    hunger = {'before': food, 'after': fed}
    hurt = {}
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    x, y, z = hx + 3, hy, hz
    for name, fill, ticks in (('lava', 'minecraft:lava', 60), ('wall', 'minecraft:stone', 60),
                              ('drowning', 'minecraft:water', 500), ('void', None, 60)):
        if remote(guest)['health'] <= 0:
            guest.raw('respawn')
            guest.wait(40)
        # A stone box around the guest holds the lava and the water in.
        guest('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:stone')
        guest('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:air')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        guest.wait(100)
        before = remote(guest)['health']
        if fill:
            guest('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} {fill}')
        else:
            guest('command', command=f'tp @s {x + 0.5} -140 {z + 0.5}')
        hurt[name] = (before, first_hurt(guest, before, ticks))
        guest.shot(out, f'hazards-{name}')
        # Water puts the lava's fire out.
        guest('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:water')
        guest.wait(5)
        guest('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        guest('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone')
    guest.wait(20)
    if remote(guest)['health'] <= 0:
        guest.raw('respawn')
        guest.wait(40)
    # [food, saturation, exhaustion]: 8 s of sprint-jumping is about 2 of exhaustion; every 4
    # of it takes a point of saturation, or of food once saturation is gone (FoodData.tick).
    spent = 4 * (food[0] + food[1] - fed[0] - fed[1]) + fed[2] - food[2]
    assert spent > 1.0, f'no hunger from moving: {hunger} ({spent:.2f} spent)'
    for name, (before, after) in hurt.items():
        assert after < before, f'{name} never hurt the guest: health {before} -> {after} (all: {hurt})'
    return {'hunger': hunger, **hurt}


def check_pvp(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:iron_sword')
    face_each_other(host, guest)
    hits = {}
    try:
        for name, rule, attacker in (('guest hits host', 'true', guest), ('host hits guest', 'true', host),
                                     ('pvp off', 'false', guest)):
            host('command', command=f'gamerule pvp {rule}')
            face_each_other(host, guest)
            attacker.wait(30)  # a full attack charge
            victim = (lambda: host('state')['player']['health']) if attacker is guest else (lambda: remote(guest)['health'])
            before = victim()
            attacker.raw('attack')
            attacker.wait(10)
            hits[name] = (before, victim())
            attacker.shot(out, 'pvp-' + name.replace(' ', '-'))
    finally:
        host('command', command='gamerule pvp true')
    assert hits['guest hits host'][1] < hits['guest hits host'][0], f'the guest did not hurt the host: {hits}'
    assert hits['host hits guest'][1] < hits['host hits guest'][0], f'the host did not hurt the guest: {hits}'
    assert hits['pvp off'][1] == hits['pvp off'][0], f'pvp off still hurts: {hits}'
    return hits


def container_items(game):
    """The open container's stacks by slot (Horizon's container slots are 300 and up) and its size."""
    slots = [s for s in game('screen').get('slots', []) if s.get('slot', 0) >= 300]
    return {s['slot'] - 300: s['item'] for s in slots if s.get('item')}, len(slots)


def open_container(guest, pos, out, label, take=None):
    """Open the container at `pos` and read it; `take` shift-clicks that container slot out first."""
    guest.raw('interact', *pos)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    # The server's menu (an ender or double chest has no stand-in block entity: key i32::MIN),
    # then its contents, can come a few ticks after the screen.
    for _ in range(20):
        guest.wait(2)
        screen = guest('screen')
        items = {s['slot']: s.get('item') for s in screen.get('slots', [])}
        if screen.get('container') == [-2**31, 0, 0] and (take is None or items.get(300 + take)):
            break
    guest.wait(5)
    if take is not None:
        guest.raw('click-slot', 300 + take, 'shift')
        guest.wait(5)
    items = container_items(guest)
    guest.shot(out, label)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    return items


def check_containers(host, guest, out):
    host('command', command='item replace entity @s enderchest.0 with minecraft:diamond')
    guest('command', command='item replace entity @s enderchest.4 with minecraft:emerald')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {x} {y} {z + 2} minecraft:ender_chest')
    guest.wait(10)
    ender, ender_size = open_container(guest, (x, y, z + 2), out, 'ender-chest')
    # Facing north, a left half pairs with the block to its east (ChestBlock.getConnectedDirection).
    guest('command', command=f'setblock {x} {y} {z - 2} minecraft:chest[facing=north,type=left]')
    guest('command', command=f'setblock {x + 1} {y} {z - 2} minecraft:chest[facing=north,type=right]')
    guest('command', command=f'item replace block {x + 1} {y} {z - 2} container.0 with minecraft:gold_ingot')
    guest('command', command=f'item replace block {x} {y} {z - 2} container.0 with minecraft:iron_ingot')
    guest.wait(10)
    double, double_size = open_container(guest, (x, y, z - 2), out, 'double-chest')
    # Taking from each writes back to the guest's ender inventory and to the right half.
    guest('command', command='clear @s')
    open_container(guest, (x, y, z + 2), out, 'ender-chest-take', take=4)
    open_container(guest, (x, y, z - 2), out, 'double-chest-take', take=0)
    taken = guest('inventory')['counts']
    after = (open_container(guest, (x, y, z + 2), out, 'ender-chest-after')[0],
             open_container(guest, (x, y, z - 2), out, 'double-chest-after')[0])
    host_ender = host('command', command='data get entity @s EnderItems')
    for at in ((x, y, z + 2), (x, y, z - 2), (x + 1, y, z - 2)):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    assert ender == {4: 'minecraft:emerald'} and ender_size == 27, f'the guest\'s ender chest: {ender} ({ender_size})'
    assert double_size == 54, f'the double chest opened with {double_size} slots'
    assert double == {0: 'minecraft:gold_ingot', 27: 'minecraft:iron_ingot'}, f'the double chest: {double}'
    assert 'emerald' not in json.dumps(host_ender), f'the guest\'s item went to the host: {host_ender}'
    assert taken.get('minecraft:emerald') == 1 and taken.get('minecraft:gold_ingot') == 1, f'taken: {taken}'
    assert after == ({}, {27: 'minecraft:iron_ingot'}), f'after taking: {after}'
    return {'ender': ender, 'double': double, 'taken': taken}


def check_feedback(host, guest, out):
    x, y, z = (round(v, 1) for v in pos_of(guest))
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:stick')
    guest('command', command=f'tp @s {x:.1f} {y:.1f} {z:.1f}')
    guest.wait(10)
    chat = guest.raw('messages', 5)['chat']
    guest.raw('key', 't', 'tap')
    guest.wait(5)
    guest.shot(out, 'feedback')
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    screen = guest('state').get('screen')
    replaced = f'Replaced 1 slot(s) on {GUEST_NAME} with [Stick]'
    assert any(replaced in line for line in chat), f'no "{replaced}" in {chat}'
    assert any(line.startswith(f'Teleported {GUEST_NAME} to {x:.6f}') for line in chat), f'teleport feedback: {chat}'
    assert screen in (None, 'playing'), f'the chat did not close: {screen}'
    return chat


def png_rows(path):
    """The rows of an 8-bit RGB or RGBA PNG, unfiltered, as bytes of 4 (or 3) per pixel."""
    data = Path(path).read_bytes()
    pos, idat, head = 8, b'', None
    while pos < len(data):
        size = int.from_bytes(data[pos:pos + 4], 'big')
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + size]
        if kind == b'IHDR':
            head = body
        elif kind == b'IDAT':
            idat += body
        pos += 12 + size
    width, height = int.from_bytes(head[:4], 'big'), int.from_bytes(head[4:8], 'big')
    assert head[8] == 8 and head[9] in (2, 6) and head[12] == 0, f'unsupported PNG {head.hex()}'
    bpp = 4 if head[9] == 6 else 3
    raw, stride, rows, prev = zlib.decompress(idat), width * bpp, [], bytearray(width * bpp)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return rows, bpp


def changed_pixels(a, b):
    (ra, bpp), (rb, _) = png_rows(a), png_rows(b)
    return sum(1 for la, lb in zip(ra, rb) for i in range(0, len(la), bpp)
               if max(abs(la[i + k] - lb[i + k]) for k in range(3)) > 24)


def sneaking_on_host(host, want, ticks=40):
    """Whether the host sees the guest crouching, once it matches `want` or after `ticks`."""
    for _ in range(ticks // 5):
        sneaking = host.raw('lan', 'status')['guests'][0]['sneaking']
        if sneaking == want:
            return sneaking
        host.wait(5)
    return sneaking


def check_walls(host, guest, out):
    # A stone wall between the two: the host still sees the guest's name through it
    # (TEXT_SEE_THROUGH), but not once the guest crouches (Entity.isDiscrete).
    for g in (host, guest):
        back_to_game(g)
    # The trial spawner's zombies may have killed the guest.
    if guest('state').get('screen') == 'dead' or remote(guest)['health'] <= 0:
        guest.raw('respawn')
        guest.wait(40)
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    gx, gy, gz = pos_of(guest)
    # Halfway: the players stand 3 apart, so the wall's block touches neither.
    wall = int((hx + gx) / 2 // 1)
    host('command', command=f'fill {wall} {int(hy // 1)} {int(hz // 1) - 3} {wall} {int(hy // 1) + 4} '
                            f'{int(hz // 1) + 3} minecraft:stone')
    try:
        host.raw('look-at', f'{gx:.2f}', f'{gy + 2.3:.2f}', f'{gz:.2f}')
        # Chat lines fade out after 200 ticks; let them go so only the name tag can differ.
        host.wait(210)
        standing = host.shot(out, 'walls-standing')
        guest.raw('key', 'shift', 'down')
        crouched = sneaking_on_host(host, True)
        guest.wait(10)
        crouching = host.shot(out, 'walls-crouching')
        guest.raw('key', 'shift', 'up')
        stood = not sneaking_on_host(host, False)
        guest.wait(10)
        again = host.shot(out, 'walls-standing-again')
    finally:
        guest.raw('key', 'shift', 'up')
        host('command', command=f'fill {wall} {int(hy // 1)} {int(hz // 1) - 3} {wall} {int(hy // 1) + 4} '
                                f'{int(hz // 1) + 3} minecraft:air')
        face_each_other(host, guest)
    assert crouched and stood, f'the host never saw the guest crouch and stand: {crouched}, {stood}'
    seen, steady = changed_pixels(standing, crouching), changed_pixels(standing, again)
    assert seen > 50, f'no name tag through the wall: {seen} pixels differ standing vs crouching'
    assert steady < seen // 4, f'the view itself changed: {steady} pixels between two standing shots ({seen})'
    return {'standing_vs_crouching': seen, 'standing_twice': steady}


def state_of(game, pos, key):
    state = game.raw('block', *pos)['state']
    return dict(kv.split('=') for kv in state.partition('[')[2].rstrip(']').split(',') if kv).get(key)


def wait_state(game, pos, key, want, ticks):
    for _ in range(ticks // 10):
        if state_of(game, pos, key) == want:
            return want
        game.wait(10)
    return state_of(game, pos, key)


def check_trials(host, guest, out):
    # The host watches as a spectator, whom neither block detects: only the guest can wake them.
    hx, hy, hz = pos_of(host)
    host('command', command='gamemode spectator')
    host('command', command='difficulty normal')
    host('command', command='gamerule spawn_mobs true')
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    spawner, vault = (x, y, z + 3), (x, y, z - 2)
    # Open air from the vault to the spawner: the spawner wants to see the guest's eyes.
    guest('command', command=f'fill {x - 1} {y} {z - 2} {x + 1} {y + 2} {z + 3} minecraft:air')
    try:
        guest('command', command=f'setblock {spawner[0]} {spawner[1]} {spawner[2]} minecraft:trial_spawner'
                                 '{spawn_data:{entity:{id:"minecraft:zombie"}},required_player_range:6}')
        guest('command', command=f'setblock {vault[0]} {vault[1]} {vault[2]} minecraft:vault'
                                 '{config:{key_item:{id:"minecraft:trial_key",count:1}}}')
        trial = wait_state(host, spawner, 'trial_spawner_state', 'active', 100)
        active = wait_state(host, vault, 'vault_state', 'active', 60)
        host.shot(out, 'trials-active')
        guest('command', command='item replace entity @s weapon.mainhand with minecraft:trial_key')
        guest.wait(5)
        guest.raw('interact', *vault)
        unlocking = wait_state(host, vault, 'vault_state', 'unlocking', 20)
        guest.shot(out, 'trials-unlocking')
        rewarded = host('command', command=f'data get block {vault[0]} {vault[1]} {vault[2]} server_data.rewarded_players')
        host_uuid = host('command', command='data get entity @s UUID')
    finally:
        host('command', command='gamerule spawn_mobs false')
        host('command', command='kill @e[type=minecraft:zombie]')
        host('command', command='gamemode survival')
        # Back where it stood: a spectator drifts, and may be inside the ground when it lands.
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
        guest('command', command=f'setblock {spawner[0]} {spawner[1]} {spawner[2]} minecraft:air')
        guest('command', command=f'setblock {vault[0]} {vault[1]} {vault[2]} minecraft:air')
    uuid = lambda reply: re.findall(r'-?\d+', json.dumps(reply).partition('[I;')[2].partition(']')[0])
    assert trial == 'active', f'the trial spawner never saw the guest: {trial}'
    assert active == 'active', f'the vault never saw the guest: {active}'
    assert unlocking == 'unlocking', f'the guest\'s key did not unlock the vault: {unlocking}'
    assert uuid(rewarded) and uuid(rewarded) != uuid(host_uuid), f'rewarded {rewarded}, host {host_uuid}'
    return {'spawner': trial, 'vault': active, 'rewarded': uuid(rewarded), 'host': uuid(host_uuid)}


def check_push(host, guest, out):
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) + 0.5 for v in pos_of(guest))
    guest('command', command=f'tp @s {x} {y - 0.5} {z}')
    guest.wait(20)
    x, y, z = pos_of(guest)
    # A cow that doesn't walk, 0.3 east of the guest: the two boxes overlap until their centres are
    # 0.75 apart, so the push stops a fraction of a block away.
    guest('command', command=f'summon minecraft:cow {x + 0.3:.2f} {y:.2f} {z:.2f} {{NoAI:1b}}')
    guest.wait(60)
    nx, _, nz = pos_of(guest)
    guest.shot(out, 'push')
    guest('command', command='kill @e[type=minecraft:cow]')
    moved = ((nx - x) ** 2 + (nz - z) ** 2) ** 0.5
    detail = {'from': [round(x, 3), round(z, 3)], 'to': [round(nx, 3), round(nz, 3)], 'moved': round(moved, 3)}
    assert nx < x, f'the cow did not push the guest west: {detail}'
    assert moved < 1.5, f'the cow pushed the guest too far: {detail}'
    return detail


CHECK_FNS = {'tab': check_tab, 'self': check_self, 'armour': check_armour, 'nametags': check_nametags,
             'chest': check_chest, 'crafting': check_crafting, 'damage': check_damage,
             'mobs': check_mobs, 'hazards': check_hazards, 'pvp': check_pvp,
             'containers': check_containers, 'feedback': check_feedback, 'trials': check_trials,
             'walls': check_walls,
             'push': check_push}


def run(ctl, out, resources, checks):
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    results = {}
    try:
        host('launch', wait=False, offscreen=sys.platform == 'darwin')
        # Guests' commands need the world's Allow Commands as well as the LAN's (#372): a creative
        # world has them; everyone plays in survival.
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        host('command', command='defaultgamemode survival')
        host('command', command='gamemode survival')
        host('command', command='time set day')
        # An open stone floor around the host, so the players see each other and nothing is in the way.
        host('command', command='fill ~-8 ~-1 ~-8 ~8 ~-1 ~8 minecraft:stone')
        host('command', command='fill ~-8 ~ ~-8 ~8 ~6 ~8 minecraft:air')
        host('command', command='gamerule spawn_mobs false')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        host.player = host.raw('lan', 'status').get('host', 'Player')  # the host's profile name
        guest('launch', wait=False, offscreen=sys.platform == 'darwin')
        guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        face_each_other(host, guest)
        host.shot(out, 'joined')
        guest.shot(out, 'joined')
        for name in checks:
            try:
                results[name] = {'pass': True, 'detail': CHECK_FNS[name](host, guest, out)}
            except (Failed, AssertionError, StopIteration, KeyError) as e:
                results[name] = {'pass': False, 'detail': repr(e)}
                for g in (host, guest):
                    back_to_game(g)
            print(f'[lan] {name}: {"PASS" if results[name]["pass"] else "FAIL"} {results[name]["detail"]}')
    finally:
        logs = {g.name: g.close() for g in (guest, host)}
        for name, text in logs.items():
            (out / f'{name}.log').write_text(text)
        for g in (host, guest):
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build with the fast profile)')
    parser.add_argument('--check', action='append', choices=CHECKS, help='run only these checks (repeatable)')
    parser.add_argument('--out', default=str(ROOT / '.local/lan-e2e'), help='screenshots and game logs go here')
    args = parser.parse_args()
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--bins'],
                       cwd=ROOT, check=True)
        meta = json.loads(subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT,
                                         capture_output=True, text=True, check=True).stdout)
        bins = Path(meta['target_directory']) / 'fast'
    resources = (ROOT / '.local/horizon-client/26.3').resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    results = run(bins / 'horizon-ctl', out, resources, args.check or CHECKS)
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
