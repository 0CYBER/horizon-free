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
             closes on both when the guest closes it; the host keeps playing with its mouse look (#687);
- crafting:  the guest crafts planks in the inventory's 2x2 grid and opens a crafting table, which
             leaves the host's screen and mouse look alone;
- sign:      a sign the guest places doesn't open a sign editor on the host;
- damage:    a guest in survival takes fall damage;
- mobs:      with the host in creative, a zombie and a skeleton go for the guest and hurt them;
- hazards:   a guest in survival gets hungry sprint-jumping and is hurt by lava, a block at the eyes,
             drowning and the void (then respawns);
- pvp:       the guest and the host hit each other with a sword; with the pvp game rule off, or on one
             team without friendly fire, nothing;
- kills:     a pig the guest kills counts in the guest's killed statistic, not the host's (#484);


- arrows:    the guest and the host shoot each other with a bow; with the pvp game rule off, nothing (#484);
- respawn:   a player killed is drawn dead by the other side, and alive again once it respawns (the
             respawned player is a new entity to the others); the host dies, and on the server thread
             the guest too;
- hurt:      a pig the guest hits, and one the host hits, flashes red on the other side too (the
             server's damage event) (#848);
- containers: the guest's ender chest holds the guest's items, not the host's, and a double chest
             opens as 9x6 with the right half first;
- machines:  a hopper, a dispenser and a dropper open on the guest's screen with their 5 and 9
             slots, and keep what the guest shift-clicks into them (#848);
- brewing:   a brewing stand opens on the guest's screen; blaze powder, nether wart and a bottle the
             guest shift-clicks go to the fuel, ingredient and bottle slots, and the stand's fuel shows
             (#848);
- feedback:  the guest's command feedback names the guest and shows the numbers;
- chat:      a line the host sends reaches the guest's event log (`wait-for chat`), and on the
             server thread the host window's too;
- effects:   a beacon's power reaches every player in range, the host and the guest;
- distance:  (--server-thread) the server follows the host's render distance, past vanilla's 32 too
             (IntegratedServer.tickServer), and the guest's chunks follow the server's (#804)
- portals:   (--server-thread) the server holds a level per dimension: the guest goes to the Nether
             through a portal while the host stays, then the host goes to the End while the guest is in
             the Nether, and both come back.
- commands:  the guest's chat completes commands and their arguments over the server's command
             tree (Tab), and runs what it completed;
- recipes:   the guest's recipe book lists the planks recipe the logs unlock, and clicking it
             places a log in the inventory's crafting grid (the server places it).
- feedback:  the guest's command feedback names the guest and shows the numbers, and the host (an
             operator) is told of it as `[Guest: ...]`.
- clicks:    with real mouse presses (not click-slot, which skips the screen's press and drag
             handling), the guest puts a stack into a chest and a furnace, drags planks across
             its crafting grid and gathers a stack with a double click; the server has each.
- sleep:     at night the guest lies down in its own bed (not the host); alone it stays night, it
             gets up with Escape, and once the host sleeps too the night ends and both wake.
- predict:   with the host frozen, the oak door the guest opens opens on the guest's screen at once (its
             prediction) and creaks there at once, stays open once the server answers, and each side hears
             it once; an iron door stays shut (#869).
- push:      a cow standing in the guest pushes it just clear of the cow (the guest's own client
             pushes it, as in vanilla), not ever faster across the ground.
- eat:       holding right click with bread, the guest's own screen plays the eating (the item in use
             counts up) and the server eats one loaf and feeds the guest.
- eating:    each side sees the other player eat: the item in use (LivingEntity's flags) on the
             player it draws, and no longer once the bread is eaten (#803);
- xp:        experience orbs 4 blocks away fly to the guest, show on the guest's screen and give the
             guest their points (not the host).
- objects:   boats, minecarts, armor stands, item frames and paintings the host summons reach the
             guest as the host has them (kind, place, rotation, what they hold or wear, the variant).
- projectiles: primed TNT (one falling), an end crystal and an arrow that lands reach the guest where
             the host has them; a falling block lands on both sides.
- fireball:  a fire charge a dispenser shoots onto stone lights a fire both sides see, and the fire
             goes out on its own within a minute (it ticks: FireBlock.tick on a floor that doesn't burn)


- blast:     primed TNT going off beside a guest in survival hurts the guest and pushes it away
             (ServerExplosion.hurtEntities takes every player); facing it behind a raised shield, the
             guest takes nothing.


- worn:      iron armour the guest puts on in its inventory protects it: a 10-damage blow takes 6
             (armour 15, LivingEntity.getDamageAfterArmorAbsorb), and the full 10 without it.
- plates:    a stone pressure plate the guest steps on is pressed on both sides, and comes up once
             the guest steps off (#848).
- swings:    each side sees the other's arm swing while it mines a block, and the guest sees its own
             swing when its hoe tills dirt (the server's use) (#848).
- cracks:    each side sees the other player's crack on the block they mine grow, and go once it breaks
             (#1096).
- time:      the guest's clock is the host's: it stops with `/time pause`, and catches up with the
             server's game time after the guest's game stalls for a few seconds (#848).
- growth:    far from the host, the chunks around the guest tick (#890): water placed there flows
             and wheat on farmland grows (random ticks), however the host runs its world.
- flight:    a player in creative double-taps jump and flies, stays up, and stops flying in survival:
             the guest, and on the server thread the host's window too, take what may fly from the
             server's abilities (ClientPacketListener.handlePlayerAbilities).
- light:     the guest's light matches the host's: around a torch placed near it, under a roof and
             once a block of the roof is removed under the sky, and in the last chunk it tracks next to
             glowstone in a chunk it doesn't (only the server's light updates bring that one).

Each check is reported on its own (a failing one doesn't stop the others) and the screenshots of
both sides go to --out (host-*.png, guest-*.png).

    python3 tools/lan_e2e.py                          # build with the fast profile, run every check
    python3 tools/lan_e2e.py --bin-dir target/fast    # prebuilt binaries
    python3 tools/lan_e2e.py --check chest --check tab
    python3 tools/lan_e2e.py --server-thread --check portals   # the host's world on its server thread

Needs the prepared resources in .local/horizon-client and, on Linux, vscreen (the desktop); on
macOS both games render offscreen. Games started here have no audio (HORIZON_NO_AUDIO=1).
"""
import argparse
import json
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_control_e2e import Cli, Failed, game_env  # noqa: E402
import lab  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
GUEST_NAME = 'Guest'
# The host plays single player on its server thread (--server-thread).
SERVER_THREAD = False
CHECKS = ['tab', 'self', 'armour', 'nametags', 'walls', 'chest', 'crafting', 'sign', 'damage', 'mobs', 'hazards', 'pvp', 'arrows', 'kills', 'hurt', 'respawn', 'containers', 'feedback', 'titles', 'chat', 'commands', 'recipes', 'machines', 'brewing', 'trials', 'clicks', 'sleep', 'predict', 'push', 'eat', 'eating', 'xp', 'objects', 'projectiles', 'worn', 'blast', 'barter', 'deflect', 'fireball', 'light', 'time', 'swings', 'cracks', 'plates', 'effects', 'portals', 'distance', 'growth', 'flight']


class Game:
    """One client: its scratch folder, its socket and a CLI driver."""

    def __init__(self, name, ctl, resources):
        self.name = name
        self.work = Path(tempfile.mkdtemp(prefix=f'hzlan-{name}-'))
        (self.work / '.local/horizon-client').mkdir(parents=True)
        os.symlink(resources, self.work / '.local/horizon-client' / lab.VERSION)
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


def dead(guest):
    """Whether the guest is dead: its death screen (the last health it was sent can stay above 0)."""
    return guest('state').get('screen') == 'dead' or remote(guest)['health'] <= 0


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
    if SERVER_THREAD:
        # The host's window is a client of its server thread: what it draws is what it was sent.
        seen = [e for e in remote(host).get('player_equipment', [])]
        assert all(any(item in json.dumps(e) for e in seen) for item in
                   ('diamond_chestplate', 'diamond_leggings')), f'the host draws the guest in {seen}'
    else:
        seen = lan_guest(host).get('model_armor')
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


def grab(game):
    """Mouse look on, as a player's grabbed pointer: an agent's mouse-move stands in for it."""
    game.raw('mouse-move', 1, 0)
    game.raw('mouse-move', -1, 0)


def host_unaffected(host, what):
    """What a guest opens is the guest's: the host keeps its world on screen and its mouse look (#687)."""
    state = host('state')
    assert state.get('screen') == 'playing', f'{what} opened the {state.get("screen")} screen on the host'
    assert state.get('pointer_grabbed'), f"{what} released the host's pointer"


def lid(game, pos):
    return game.raw('block', *pos).get('lid', 0.0)


def check_chest(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest = (x, y, z + 2)
    guest('command', command=f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest')
    guest.wait(10)
    grab(host)
    guest.raw('interact', *chest)
    screen = guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(20)
    host_unaffected(host, "the guest's chest")
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
    grab(host)
    guest.raw('interact', *table)
    guest.raw('wait-for', 'screen', 'inventory', '--timeout', 10)
    host_unaffected(host, "the guest's crafting table")
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


def check_sign(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(guest))
    spot = (x + 2, y, z)
    guest('command', command=f'setblock {spot[0]} {spot[1] - 1} {spot[2]} minecraft:stone')
    guest('command', command=f'setblock {spot[0]} {spot[1]} {spot[2]} minecraft:air')
    guest('command', command='give @s minecraft:oak_sign 1')
    guest.wait(10)
    grab(host)
    guest.raw('place', *spot, '--item', 'minecraft:oak_sign')
    guest.wait(20)
    placed = host.raw('block', *spot).get('block')
    host.shot(out, 'sign')
    assert placed and 'sign' in placed, f'the guest placed {placed}'
    host_unaffected(host, "the guest's sign")
    return placed


def effect_ids(game):
    return [e.get('id', '').removeprefix('minecraft:') for e in game('state')['player'].get('effects', [])]


def check_effects(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    bx, by, bz = x - 3, y, z - 3
    host('command', command=f'fill {bx - 1} {by - 1} {bz - 1} {bx + 1} {by - 1} {bz + 1} minecraft:iron_block')
    host('command', command=f'setblock {bx} {by} {bz} minecraft:beacon{{primary_effect:"minecraft:speed"}}')
    # BeaconBlockEntity.tick applies its powers every 80 ticks, once the beam has reached the sky.
    seen = {}
    for _ in range(15):
        host.wait(20)
        seen = {'host': effect_ids(host), 'guest': effect_ids(guest)}
        if all('speed' in v for v in seen.values()):
            break
    host.shot(out, 'effects')
    guest.shot(out, 'effects')
    assert 'speed' in seen['host'] and 'speed' in seen['guest'], f'beacon effects: {seen}'
    return seen


def wait_dimension(game, dimension, ticks):
    """Wait up to `ticks` for the game to be in `dimension` with its world loaded around it."""
    for _ in range(ticks // 20):
        state = game('state')
        if state.get('dimension') == dimension and state.get('player'):
            return state
        game.wait(20)
    return game('state')


def nether_portal(game, x, y, z):
    """A lit portal along x, two blocks wide and three tall, its inside starting at x y z."""
    game('command', command=f'fill {x - 1} {y - 1} {z} {x + 2} {y + 3} {z} minecraft:obsidian')
    game('command', command=f'fill {x} {y} {z} {x + 1} {y + 2} {z} minecraft:nether_portal[axis=x]')


def lan_guest(host):
    """The guest's entry in the host's `lan status` (on the server thread the owner is listed too)."""
    guests = host.raw('lan', 'status')['guests']
    return next(g for g in guests if g['name'] == GUEST_NAME)

def check_distance(host, guest, out):
    """IntegratedServer.tickServer: the server takes the host's render distance as its view distance
    (past 32, which Horizon's options allow) and sends it to every player (SetChunkCacheRadius)."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    seen = {}
    try:
        for distance in (48, 6):
            host.raw('render-distance', distance)
            # The pause screen doesn't hold the server here: it follows on its next tick.
            host.wait(20)
            guest.wait(20)
            seen[distance] = (host('status').get('render_distance'), guest('status').get('render_distance'))
    finally:
        host.raw('render-distance', 12)
    # A client streams out to min(its render distance, the server's view distance + 1).
    assert seen[48][0] == 48, f'the host at 48 streams {seen[48][0]} chunks out (all: {seen})'
    assert seen[6][0] == 6, f'the host at 6 streams {seen[6][0]} chunks out (all: {seen})'
    assert seen[6][1] == 7, f'the guest streams {seen[6][1]} with the server at 6 (all: {seen})'
    return seen


def check_portals(host, guest, out):
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    x, y, z = (int(v // 1) for v in pos_of(guest))
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    try:
        return portals(host, guest, out, x, y, z, hx, hy, hz)
    finally:
        # The guest comes home into its portal's frame (an obsidian ceiling three blocks up) and the
        # host onto its End portal at spawn: take both away so the next checks start in the open.
        home = 'execute in minecraft:overworld run'
        guest('command', command=f'{home} fill {x - 1} {y} {z + 3} {x + 2} {y + 3} {z + 3} minecraft:air')
        host('command', command=f'{home} setblock {hx} {hy} {hz} minecraft:air')
        guest('command', command=f'{home} tp @s {x + 0.5} {y} {z + 0.5}')
        guest.wait(20)


def portals(host, guest, out, x, y, z, hx, hy, hz):
    nether_portal(guest, x, y, z + 3)
    guest.wait(5)
    guest('command', command=f'tp @s {x + 0.5} {y} {z + 3.5}')
    # players_nether_portal_default_delay: 80 ticks in survival.
    there = wait_dimension(guest, 'minecraft:the_nether', 600)
    guest.wait(40)
    guest.shot(out, 'portals-nether')
    host_dim = host('state').get('dimension')
    assert there.get('dimension') == 'minecraft:the_nether', f'the guest is in {there.get("dimension")}'
    assert host_dim == 'minecraft:overworld', f'the host went to {host_dim} with the guest'
    arrived = [int(v // 1) for v in pos_of(guest)]
    below = guest.raw('block', arrived[0], arrived[1] - 1, arrived[2])
    assert below.get('loaded') and below.get('block') != 'minecraft:air', f'under the guest in the Nether: {below}'
    # The host takes an End portal while the guest is in the Nether: three levels at once.
    host('command', command=f'setblock {hx} {hy} {hz} minecraft:end_portal')
    end = wait_dimension(host, 'minecraft:the_end', 600)
    host.wait(40)
    host.shot(out, 'portals-end')
    guest_dim = guest('state').get('dimension')
    assert end.get('dimension') == 'minecraft:the_end', f'the host is in {end.get("dimension")}'
    assert guest_dim == 'minecraft:the_nether', f'the guest left the Nether for {guest_dim}'
    # Both come back: the host through an exit portal, the guest out of its portal and in again.
    ex, ey, ez = (int(v // 1) for v in pos_of(host))
    host('command', command=f'setblock {ex + 2} {ey} {ez} minecraft:end_portal')
    host('command', command=f'tp @s {ex + 2.5} {ey} {ez + 0.5}')
    guest('command', command=f'tp @s {arrived[0] + 0.5} {arrived[1]} {arrived[2] - 2.5}')
    guest.wait(30)
    guest('command', command=f'tp @s {arrived[0] + 0.5} {arrived[1]} {arrived[2] + 0.5}')
    home = wait_dimension(host, 'minecraft:overworld', 600)
    back = wait_dimension(guest, 'minecraft:overworld', 600)
    guest.wait(40)
    host.shot(out, 'portals-back')
    guest.shot(out, 'portals-back')
    assert home.get('dimension') == 'minecraft:overworld', f'the host stayed in {home.get("dimension")}'
    assert back.get('dimension') == 'minecraft:overworld', f'the guest stayed in {back.get("dimension")}'
    return {'guest_nether': arrived, 'host_end': [ex, ey, ez], 'back': pos_of(guest)}


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
    # ServerPlayer.doTick sends the dead player's health of 0 too.
    died = {'screen': guest('state').get('screen'), 'health': remote(guest)['health']}
    assert died == {'screen': 'dead', 'health': 0.0}, f'the guest was killed: {died}'
    guest.raw('respawn')
    guest.wait(40)
    # Sprint-jumping down a stone lane: ServerPlayer.jumpFromGround and checkMovementStatistics.
    guest('command', command='fill ~ ~-1 ~-1 ~60 ~-1 ~1 minecraft:stone')
    guest('command', command='fill ~ ~ ~-1 ~60 ~3 ~1 minecraft:air')
    guest.wait(10)
    x, y, z = pos_of(guest)
    guest.raw('look-at', f'{x + 60:.2f}', f'{y + 1.6:.2f}', f'{z:.2f}')
    food = lan_guest(host)['food']
    for k in ('w', 'ctrl', 'space'):
        guest.raw('key', k, 'down')
    guest.wait(160)
    for k in ('space', 'ctrl', 'w'):
        guest.raw('key', k, 'up')
    guest.wait(10)
    fed = lan_guest(host)['food']
    hunger = {'before': food, 'after': fed}
    hurt = {}
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    x, y, z = hx + 3, hy, hz
    for name, fill, ticks in (('lava', 'minecraft:lava', 60), ('wall', 'minecraft:stone', 60),
                              ('drowning', 'minecraft:water', 500), ('void', None, 60)):
        if dead(guest):
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
        # The host tidies up: the void can kill the guest before this, the server drops a dead
        # guest's packets (lan_guest_packet), and the box left standing suffocated it in pvp.
        # Water puts the lava's fire out.
        host('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:water')
        guest.wait(5)
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone')
    guest.wait(20)
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    # [food, saturation, exhaustion]: 8 s of sprint-jumping is about 2 of exhaustion; every 4
    # of it takes a point of saturation, or of food once saturation is gone (FoodData.tick).
    spent = 4 * (food[0] + food[1] - fed[0] - fed[1]) + fed[2] - food[2]
    assert spent > 1.0, f'no hunger from moving: {hunger} ({spent:.2f} spent)'
    for name, (before, after) in hurt.items():
        assert after < before, f'{name} never hurt the guest: health {before} -> {after} (all: {hurt})'
    return {'died': died, 'hunger': hunger, **hurt}


def check_pvp(host, guest, out):
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:iron_sword')
    face_each_other(host, guest)
    hits = {}
    try:
        for name, rule, attacker in (('guest hits host', 'true', guest), ('host hits guest', 'true', host),
                                     ('pvp off', 'false', guest), ('guest hits teammate', 'true', guest),
                                     ('host hits teammate', 'true', host)):
            host('command', command=f'gamerule pvp {rule}')
            if name == 'guest hits teammate':
                # Player.canHarmPlayer: the victim's team without friendly fire spares its allies.
                host('command', command='team add parity_allies')
                host('command', command='team modify parity_allies friendlyFire false')
                host('command', command='team join parity_allies @a')
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
        host('command', command='team remove parity_allies')
    assert hits['guest hits host'][1] < hits['guest hits host'][0], f'the guest did not hurt the host: {hits}'
    assert hits['host hits guest'][1] < hits['host hits guest'][0], f'the host did not hurt the guest: {hits}'
    # Natural regeneration may heal the victim meanwhile.
    assert hits['pvp off'][1] >= hits['pvp off'][0], f'pvp off still hurts: {hits}'
    for name in ('guest hits teammate', 'host hits teammate'):
        assert hits[name][1] == hits[name][0], f'friendly fire off still hurts ({name}): {hits}'
    return hits


def check_kills(host, guest, out):
    # A mob the guest kills is the guest's kill (#484): LivingEntity.die credits lastHurtByPlayer, so the
    # killed statistic (and its scoreboard criterion) counts for the guest, not the host.
    host('command', command='scoreboard objectives add parity_kills minecraft.killed:minecraft.pig')
    for g in (host, guest):
        g('command', command='gamemode survival')
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:iron_sword')
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    try:
        # Between the two, in front of the guest, with one health left.
        host('command', command=f'summon minecraft:pig {hx + 1.5:.2f} {hy:.2f} {hz:.2f} {{NoAI:1b,Health:1.0f}}')
        guest.wait(30)
        guest.raw('look-at', f'{hx + 1.5:.2f}', f'{hy + 0.5:.2f}', f'{hz:.2f}')
        guest.wait(10)
        guest.raw('attack')
        guest.wait(20)
        host('command', command=f'scoreboard players get {GUEST_NAME} parity_kills')
        host('command', command=f'scoreboard players get {host.player} parity_kills')
        host.wait(10)
        chat = host.raw('messages', 10)['chat']
        guest.shot(out, 'kills')
    finally:
        host('command', command='kill @e[type=minecraft:pig]')
        host('command', command='scoreboard objectives remove parity_kills')
    assert any(f'{GUEST_NAME} has 1 [parity_kills]' in line for line in chat), f"the guest's kill is not theirs: {chat}"
    assert not any(f'{host.player} has 1 [parity_kills]' in line for line in chat), f"the host got the guest's kill: {chat}"
    return chat


def check_arrows(host, guest, out):
    # A player's arrow on another player (#484): AbstractArrow.onHitEntity, refused by
    # ServerPlayer.canHarmPlayer with the pvp game rule off (the arrow turns back).
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:bow')
        g('command', command='give @s minecraft:arrow 16')
    hits = {}
    try:
        for name, rule, shooter in (('guest shoots host', 'true', guest), ('host shoots guest', 'true', host),
                                    ('pvp off', 'false', guest)):
            host('command', command=f'gamerule pvp {rule}')
            for g in (host, guest):
                g('command', command='effect give @s minecraft:instant_health 1 5')
            face_each_other(host, guest)
            shooter.wait(20)
            victim = (lambda: host('state')['player']['health']) if shooter is guest else (lambda: remote(guest)['health'])
            before = victim()
            shooter.raw('use-item', '--hold', 1.3)  # a full draw (BowItem: 20 ticks)
            shooter.wait(20)
            hits[name] = (before, victim())
            shooter.shot(out, 'arrows-' + name.replace(' ', '-'))
    finally:
        host('command', command='gamerule pvp true')
        for g in (host, guest):
            g('command', command='clear @s')
    assert hits['guest shoots host'][1] < hits['guest shoots host'][0], f"the guest's arrow did not hurt the host: {hits}"
    assert hits['host shoots guest'][1] < hits['host shoots guest'][0], f"the host's arrow did not hurt the guest: {hits}"
    assert hits['pvp off'][1] >= hits['pvp off'][0], f'pvp off, an arrow still hurts: {hits}'
    return hits


def dead_players(game, want, ticks=60):
    """The other players `game`'s client draws dead, once that is `want` (empty or not) or after `ticks`."""
    for _ in range(ticks // 5):
        dead = remote(game)['dead_players']
        if bool(dead) == want:
            return dead
        game.wait(5)
    return dead


def check_respawn(host, guest, out):
    # ServerLevel.removePlayerImmediately and addRespawnedPlayer: a respawned player is removed
    # from the other players and paired again as a new entity, so their clients drop the body.
    # The host window only has a client's view of the guest on the server thread.
    for g in (host, guest):
        back_to_game(g)
    results = {}
    sides = [('host', host, guest)] + ([('guest', guest, host)] if SERVER_THREAD else [])
    for name, dying, watching in sides:
        face_each_other(host, guest)
        dying('command', command='kill @s')
        dying.wait(30)
        dead = dead_players(watching, True)
        watching.shot(out, f'respawn-{name}-dead')
        dying.raw('respawn')
        dying.wait(40)
        back_to_game(dying)
        # Back within tracking range of the other player, wherever the respawn point is.
        x, y, z = pos_of(dying)
        watching('command', command=f'tp @s {x + 3:.2f} {y:.2f} {z:.2f}')
        watching.wait(40)
        alive = dead_players(watching, False)
        watching.shot(out, f'respawn-{name}-alive')
        results[name] = {'dead': dead, 'after_respawn': alive}
        assert dead, f'the {name} was not drawn dead: {results}'
        assert not alive, f'the {name} still lies dead after respawning: {results}'
    return results


def check_hurt(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    # Between the two, within both players' reach; bare hands leave it alive.
    host('command', command=f'summon minecraft:pig {hx + 1.5:.2f} {hy:.2f} {hz + 0.8:.2f} {{NoAI:1b}}')
    host.wait(30)

    def pig(game):
        return game.raw('entities', 6, 'pig')['entities'][0]
    seen = {}
    for name, attacker, watcher in (('guest hits', guest, host), ('host hits', host, guest)):
        attacker.wait(30)  # a full attack charge
        attacker.raw('attack', pig(attacker)['id'])
        seen[name] = {'attacker': pig(attacker)['hurt_time'], 'watcher': pig(watcher)['hurt_time']}
        attacker.shot(out, 'hurt-' + name.replace(' ', '-'))
        watcher.shot(out, 'hurt-' + name.replace(' ', '-') + '-seen')
        attacker.wait(20)
    host('command', command='kill @e[type=minecraft:pig]')
    # LivingEntity.hurtTime: 10 when hurt, one less each tick; the reads come a few ticks later.
    for name, sides in seen.items():
        assert all(t > 0 for t in sides.values()), f'{name}: the pig\'s hurt flash {seen}'
    return seen


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


def check_machines(host, guest, out):
    # After `recipes`: the stacks given here unlock recipes, which would move the book's first one.
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    seen = {}
    for block in ('hopper', 'dispenser', 'dropper'):
        guest('command', command='clear @s')
        guest('command', command='give @s minecraft:cobblestone 4')
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
        guest.wait(10)
        guest.raw('interact', *at)
        guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
        guest.wait(10)
        size = container_items(guest)[1]
        guest.shot(out, f'machines-{block}')
        guest.raw('click-slot', hotbar_slot(guest, 'minecraft:cobblestone'), 'shift')
        guest.wait(10)
        guest.raw('key', 'esc', 'tap')
        guest.wait(10)
        # Opened again, the menu holds what the server has.
        seen[block] = (size, open_container(guest, at, out, f'machines-{block}-after')[0])
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    want = {'hopper': (5, {0: 'minecraft:cobblestone'}), 'dispenser': (9, {0: 'minecraft:cobblestone'}),
            'dropper': (9, {0: 'minecraft:cobblestone'})}
    assert seen == want, f'the machines on the guest: {seen}'
    return seen


def check_brewing(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    loads = ('blaze_powder', 'nether_wart', 'glass_bottle')
    for item in loads:
        guest('command', command=f'give @s minecraft:{item}')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:brewing_stand')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    size = container_items(guest)[1]
    # BrewingStandMenu.quickMoveStack: blaze powder to the fuel slot, an ingredient to its slot,
    # a bottle to the bottle slots.
    for item in loads:
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    # BrewingStandBlockEntity.serverTick burns the blaze powder into 20 uses; the guest sees them
    # through the menu's data slots.
    guest.wait(10)
    info = guest.raw('menu')['info']
    guest.shot(out, 'brewing')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    items = open_container(guest, at, out, 'brewing-after')[0]
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'size': size, 'items': items, 'fuel': info and info.get('fuel')}
    want = {'size': 5, 'items': {0: 'minecraft:glass_bottle', 3: 'minecraft:nether_wart'}, 'fuel': 20}
    assert seen == want, f'the brewing stand on the guest: {seen}'
    return seen


def slot_point(game, slot):
    """The GUI point in the middle of slot `slot` of the open screen (`ui`'s slot-N)."""
    for w in game.raw('ui')['widgets']:
        if w.get('id') == f'slot-{slot}':
            x, y, width, height = w['bounds']
            return x + width / 2, y + height / 2
    raise Failed(f'no slot {slot} on the screen')


def mouse(game, *slots, right=False):
    """Press over the first slot, move over the others and release: one click, or a drag."""
    points = [f'{v:.1f}' for s in slots for v in slot_point(game, s)]
    game.raw('drag-at', *(['right'] if right else []), *points)
    game.wait(4)


def hotbar_slot(game, item):
    return next(s['slot'] for s in game('inventory')['hotbar'] if s and s.get('item') == item)


def check_clicks(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:cobblestone 10')
    guest('command', command='give @s minecraft:coal 3')
    guest('command', command='give @s minecraft:raw_iron 2')
    guest('command', command='give @s minecraft:oak_planks 2')
    guest.wait(10)
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest, furnace = (x, y, z + 2), (x, y, z - 2)
    guest('command', command=f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest')
    guest('command', command=f'setblock {furnace[0]} {furnace[1]} {furnace[2]} minecraft:furnace')
    guest.wait(10)
    # Pick the cobblestone up and put it down in the chest's first slot.
    guest.raw('interact', *chest)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    mouse(guest, hotbar_slot(guest, 'minecraft:cobblestone'))
    mouse(guest, 300)
    guest.wait(10)
    guest.shot(out, 'clicks-chest')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    in_chest, _ = open_container(guest, chest, out, 'clicks-chest-after')
    # Coal into the fuel slot, raw iron into the input.
    guest.raw('interact', *furnace)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    mouse(guest, hotbar_slot(guest, 'minecraft:coal'))
    mouse(guest, 301)
    mouse(guest, hotbar_slot(guest, 'minecraft:raw_iron'))
    mouse(guest, 300)
    guest.wait(10)
    guest.shot(out, 'clicks-furnace')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # Opening the furnace again resends the whole menu: the server's own view.
    in_furnace, _ = open_container(guest, furnace, out, 'clicks-furnace-after')
    # Two planks dragged over the 2x2 grid's left column: one each, four sticks.
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    guest.wait(5)
    mouse(guest, hotbar_slot(guest, 'minecraft:oak_planks'))
    mouse(guest, 100, 102)
    guest.wait(10)
    grid = {s['slot']: (s.get('item'), s.get('count')) for s in guest('screen').get('slots', [])
            if s.get('slot') in (100, 102, 200) and s.get('item')}
    guest.shot(out, 'clicks-drag')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(10)
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # The chest's menu brings the whole inventory as the server has it.
    open_container(guest, chest, out, 'clicks-inventory')
    counts = guest('inventory')['counts']
    for at in (chest, furnace):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    assert in_chest == {0: 'minecraft:cobblestone'}, f'the chest holds {in_chest} after the guest put cobblestone in'
    assert in_furnace == {0: 'minecraft:raw_iron', 1: 'minecraft:coal'}, f'the furnace holds {in_furnace}'
    assert grid.get(100) == ('minecraft:oak_planks', 1) and grid.get(102) == ('minecraft:oak_planks', 1), \
        f'the drag left the grid as {grid}'
    assert counts.get('minecraft:stick') == 4 and 'minecraft:cobblestone' not in counts, f'inventory after: {counts}'
    return {'chest': in_chest, 'furnace': in_furnace, 'grid': grid, 'inventory': counts}


def check_feedback(host, guest, out):
    x, y, z = (round(v, 1) for v in pos_of(guest))
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:stick')
    guest('command', command=f'tp @s {x:.1f} {y:.1f} {z:.1f}')
    guest.wait(10)
    chat = guest.raw('messages', 5)['chat']
    # The host is an operator: broadcastToAdmins tells it, as `[Guest: ...]`.
    host_chat = host.raw('messages', 5)['chat']
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
    told = f'[{GUEST_NAME}: Teleported {GUEST_NAME} to {x:.6f}'
    assert any(line.startswith(told) for line in host_chat), f'no "{told}" for the host: {host_chat}'
    return chat + host_chat


def chat_input(game):
    return game('screen').get('chat_input')


def complete(guest, text):
    """Opens the chat on `/`, types `text` and presses Tab: the input after it."""
    guest.raw('key', 'slash', 'tap')
    guest.wait(3)
    guest.raw('type', text)
    guest.wait(2)
    guest.raw('key', 'tab', 'tap')
    guest.wait(2)
    return chat_input(guest)


def check_commands(host, guest, out):
    literal = complete(guest, 'gamem')
    # The first Escape hides the suggestions, the next closes the chat.
    for _ in range(3):
        if guest('screen').get('screen') != 'chat':
            break
        guest.raw('key', 'esc', 'tap')
        guest.wait(3)
    argument = complete(guest, 'gamemode cr')
    guest.shot(out, 'commands')
    guest.raw('key', 'enter', 'tap')
    guest.wait(20)
    mode = guest('state').get('gamemode')
    screen = guest('state').get('screen')
    guest('command', command='gamemode survival')
    guest.wait(10)
    # A node the tree marks ask_server (a bossbar id): the guest asks the
    # server, which completes it from its own bossbars.
    host('command', command='bossbar add pt6:check "Check"')
    host.wait(5)
    guest.raw('key', 'slash', 'tap')
    guest.wait(3)
    guest.raw('type', 'bossbar get pt')
    guest.wait(10)
    guest.raw('key', 'tab', 'tap')
    guest.wait(2)
    asked = chat_input(guest)
    for _ in range(3):
        if guest('screen').get('screen') != 'chat':
            break
        guest.raw('key', 'esc', 'tap')
        guest.wait(3)
    host('command', command='bossbar remove pt6:check')
    assert literal == '/gamemode', f'Tab after /gamem: {literal!r}'
    assert argument == '/gamemode creative', f'Tab after /gamemode cr: {argument!r}'
    assert mode == 'creative', f'the completed command did not run: {mode}'
    assert screen in (None, 'playing'), f'the chat did not close: {screen}'
    assert asked == '/bossbar get pt6:check', f'Tab after /bossbar get pt: {asked!r}'
    return {'literal': literal, 'argument': argument, 'mode': mode, 'ask_server': asked}


def centre(rect):
    x, y, w, h = rect
    return x + w / 2, y + h / 2


def grid_items(game):
    slots = game('screen').get('slots') or []
    return {s['slot']: s.get('item') for s in slots if 100 <= s['slot'] < 109 and s.get('item')}


def check_recipes(host, guest, out):
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_log 4')
    guest.wait(40)
    guest.raw('key', 'e', 'tap')
    guest.wait(5)
    try:
        book = guest('screen').get('recipe_book')
        assert book, 'the guest\'s inventory has no recipe book'
        if len(book['targets']) == 1:
            guest.raw('click-at', *centre(book['targets'][0]))
            guest.wait(5)
            book = guest('screen').get('recipe_book')
        known = book['known']
        assert known > 0, f'the guest\'s book is empty: {book}'
        guest.raw('click-at', *centre(book['targets'][1]))
        guest.wait(10)
        grid = grid_items(guest)
        guest.shot(out, 'recipes')
    finally:
        guest.raw('key', 'esc', 'tap')
        guest.wait(5)
    assert any('oak_log' in str(item) for item in grid.values()), f'nothing placed in the grid: {grid}'
    return {'known': known, 'grid': grid}


def check_titles(host, guest, out):
    # TitleCommand sends the guest its title packets; its HUD shows and clears them.
    for command in ('title @s times 10 400 10', 'title @s subtitle "Under"', 'title @s title "Over"',
                    'title @s actionbar "Bar"'):
        guest('command', command=command)
    guest.wait(5)
    shown = {k: guest.raw('messages', 1)[k] for k in ('title', 'subtitle', 'actionbar')}
    guest.shot(out, 'titles')
    guest('command', command='title @s clear')
    guest.wait(5)
    cleared = {k: guest.raw('messages', 1)[k] for k in ('title', 'subtitle')}
    assert shown == {'title': 'Over', 'subtitle': 'Under', 'actionbar': 'Bar'}, f'titles shown: {shown}'
    assert cleared == {'title': None, 'subtitle': None}, f'titles cleared: {cleared}'
    return {'shown': shown, 'cleared': cleared}


def check_chat(host, guest, out):
    # The agent's event log follows a remote client's chat: `wait-for chat` sees the line another
    # player sends (and on the server thread the host window's own line, back from the server).
    line = f'chat check {int(time.time())}'
    host.raw('chat', line)
    seen = {g.name: g.raw('wait-for', 'chat', line, '--timeout', 10, '--since', 0).get('ok')
            for g in ((guest, host) if SERVER_THREAD else (guest,))}
    guest.shot(out, 'chat')
    return seen


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
        sneaking = lan_guest(host)['sneaking']
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
    if dead(guest):
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


def check_plates(host, guest, out):
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    plate = (x + 2, y, z)
    guest('command', command=f'setblock {plate[0]} {plate[1]} {plate[2]} minecraft:stone_pressure_plate')
    guest.wait(10)
    guest('command', command=f'tp @s {plate[0] + 0.5} {plate[1]} {plate[2] + 0.5}')
    pressed = {'host': wait_state(host, plate, 'powered', 'true', 40),
               'guest': wait_state(guest, plate, 'powered', 'true', 40)}
    guest.shot(out, 'plate-pressed')
    guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    # PressurePlateBlock.getPressedTime: a stone plate checks again every 20 ticks.
    released = {'host': wait_state(host, plate, 'powered', 'false', 60),
                'guest': wait_state(guest, plate, 'powered', 'false', 60)}
    guest('command', command=f'setblock {plate[0]} {plate[1]} {plate[2]} minecraft:air')
    detail = {'pressed': pressed, 'released': released}
    assert pressed == {'host': 'true', 'guest': 'true'}, f'the plate under the guest: {detail}'
    assert released == {'host': 'false', 'guest': 'false'}, f'the plate after the guest left: {detail}'
    return detail


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


def bed_at(game, foot):
    """A bed whose foot is at `foot`, its head one block east."""
    x, y, z = foot
    game('command', command=f'setblock {x} {y} {z} minecraft:red_bed[facing=east,part=foot]')
    game('command', command=f'setblock {x + 1} {y} {z} minecraft:red_bed[facing=east,part=head]')
    return [x + 1, y, z]


def check_sleep(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
    host('command', command='time set midnight')
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    guest_bed = bed_at(guest, (gx, gy, gz + 2))
    host_bed = bed_at(host, (hx, hy, hz + 2))
    guest.wait(10)
    guest.raw('interact', *guest_bed)
    guest.wait(10)
    asleep = {'guest': guest('state').get('sleeping'), 'host': host('state').get('sleeping'),
              'server': lan_guest(host).get('sleeping')}
    guest.shot(out, 'sleep-guest')
    # The host draws the guest lying in its bed (#1045): in frame its model, on the server thread its
    # client's (SLEEPING_POS), and a client pairing with the sleeper later too.
    host.wait(5)
    lying = {'host': lan_guest(host).get('model_lying') if not SERVER_THREAD else bool(remote(host)['lying_players'])}
    host.shot(out, 'sleep-host-sees-guest')
    # Alone in bed: still night 120 ticks later (everyone must sleep).
    guest.wait(120)
    night = host('state')['time']['time_of_day']
    # Escape gets the guest up (InBedChatScreen), then back to bed.
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    up = guest('state').get('sleeping')
    guest.raw('interact', *guest_bed)
    host.raw('interact', *host_bed)
    host.wait(5)
    # Both in bed: the guest draws the host lying too.
    lying['guest'] = bool(remote(guest)['lying_players'])
    host.wait(125)
    after = {'guest': guest('state').get('sleeping'), 'host': host('state').get('sleeping'),
             'time': host('state')['time']['time_of_day']}
    lying['after'] = bool(remote(guest)['lying_players'])
    guest.shot(out, 'sleep-morning')
    for at in (guest_bed, host_bed):
        guest('command', command=f'fill {at[0] - 1} {at[1]} {at[2]} {at[0]} {at[1]} {at[2]} minecraft:air')
    host('command', command='time set day')
    detail = {'asleep': asleep, 'night': night, 'up': up, 'after': after, 'lying': lying}
    assert asleep['guest'] == guest_bed and asleep['server'] == guest_bed, f'the guest is not in its bed: {detail}'
    assert asleep['host'] is None, f'the guest\'s bed put the host to sleep: {detail}'
    assert 13000 < night < 23000, f'one of two players asleep ended the night: {detail}'
    assert up is None, f'Escape did not get the guest up: {detail}'
    assert after['guest'] is None and after['host'] is None and after['time'] < 1000, f'no morning: {detail}'
    assert lying['host'] and lying['guest'], f'a sleeping player is drawn standing: {detail}'
    assert not lying['after'], f'a player up again is still drawn lying: {detail}'
    return detail
def check_predict(host, guest, out):
    # MultiPlayerGameMode.useItemOn under startPrediction (#869): the guest's door opens on its own screen
    # at once, with the host (the server) frozen, and the server's answer later agrees; an iron door
    # (no hand use) stays shut on both sides.
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    wood, iron = (x - 1, y, z + 2), (x + 1, y, z + 2)
    for (bx, by, bz), kind in ((wood, 'oak_door'), (iron, 'iron_door')):
        for half, dy in (('lower', 0), ('upper', 1)):
            host('command', command=f'setblock {bx} {by + dy} {bz} minecraft:{kind}[facing=north,hinge=right,half={half}]')
    guest.wait(20)
    state = lambda game, at: game.raw('block', *at)['state']
    seen = {}
    creaks = lambda game: sum(n for s, n in game('status')['heard'].items() if 'door' in s)
    heard = {'guest': creaks(guest), 'host': creaks(host)}
    os.kill(host.pid, signal.SIGSTOP)
    try:
        guest.raw('interact', *wood)
        seen['guest_frozen'] = (state(guest, wood), state(guest, (wood[0], wood[1] + 1, wood[2])))
        # level.playSound(player, ...): the clicker hears its own door at once.
        seen['guest_heard_frozen'] = creaks(guest) - heard['guest']
        guest.raw('interact', *iron)
        seen['guest_iron_frozen'] = state(guest, iron)
    finally:
        os.kill(host.pid, signal.SIGCONT)
    guest.wait(20)
    seen['guest_after'] = state(guest, wood)
    seen['host_after'] = state(host, wood)
    seen['iron_after'] = (state(guest, iron), state(host, iron))
    # ... and not a second time from the server; the host hears it once.
    seen['heard_after'] = {'guest': creaks(guest) - heard['guest'], 'host': creaks(host) - heard['host']}
    guest.shot(out, 'predict')
    for (bx, by, bz) in (wood, iron):
        host('command', command=f'fill {bx} {by} {bz} {bx} {by + 1} {bz} minecraft:air')
    assert all('open=true' in s for s in seen['guest_frozen']), f'the door waited for the server: {seen}'
    assert 'open=true' in seen['guest_after'] and 'open=true' in seen['host_after'], f'the door did not stay open: {seen}'
    assert 'open=false' in seen['guest_iron_frozen'], f'the guest opened an iron door by hand: {seen}'
    assert all('open=false' in s for s in seen['iron_after']), f'an iron door opened: {seen}'
    assert seen['guest_heard_frozen'] == 1 and seen['heard_after'] == {'guest': 1, 'host': 1}, \
        f'the door is not heard once on each side: {seen}'
    return seen


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
def check_eat(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    # Hungry: the eating needs food below 20 (Player.canEat).
    guest('command', command='effect give @s minecraft:hunger 3 255')
    guest.wait(70)
    guest('command', command='effect clear @s')
    guest('command', command='give @s minecraft:bread 3')
    guest.wait(5)
    guest.raw('select', 'minecraft:bread')
    hungry = guest('state')['player']['food']
    # use-item --hold returns once released: watch the guest's screen meanwhile.
    hold = threading.Thread(target=guest.raw, args=('use-item', '--hold', 2.2))
    hold.start()
    using = []
    for _ in range(4):
        time.sleep(0.2)
        using.append(guest('state').get('using'))
    guest.shot(out, 'eat')
    hold.join()
    guest.wait(10)
    fed = guest('state')['player']['food']
    left = guest('inventory')['counts'].get('minecraft:bread', 0)
    detail = {'hungry': hungry, 'fed': fed, 'bread': left, 'using': using, 'after': guest('state').get('using')}
    assert hungry < 20, f'the guest is not hungry: {detail}'
    ticks = [u['ticks'] for u in using if u]
    assert len(ticks) >= 3 and ticks == sorted(ticks) and ticks[-1] > ticks[0], f'no eating on the guest\'s screen: {detail}'
    assert left == 2 and fed > hungry, f'the server did not eat one loaf: {detail}'
    assert detail['after'] is None, f'still eating after the release: {detail}'
    return detail


def watch_mining(miner, pos, seen):
    """Mine the block at `pos` (by hand, for seconds) and sample `seen()` meanwhile."""
    dig = threading.Thread(target=miner.raw, args=('break', *pos))
    dig.start()
    samples = []
    for _ in range(8):
        time.sleep(0.25)
        samples.append(seen())
    dig.join()
    return samples


def check_swings(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    # Stone by hand takes 7.5 seconds: ServerboundPunchPacket every tick keeps the arm swinging.
    guest('command', command=f'setblock {gx} {gy} {gz + 1} minecraft:stone')
    host('command', command=f'setblock {hx} {hy} {hz + 1} minecraft:stone')
    host.wait(10)
    def swinging(game):
        # On its server thread the host's window is a client of its server too.
        if game is host and not SERVER_THREAD:
            return lan_guest(host).get('model_swinging')
        return any(e.get('swinging') for e in remote(game).get('player_equipment', []))
    # Most samples: a swing restarts only past its first half (SwingState.startIfAble), so a sample can
    # fall between two.
    host_sees = watch_mining(guest, (gx, gy, gz + 1), lambda: swinging(host))
    guest_sees = watch_mining(host, (hx, hy, hz + 1), lambda: swinging(guest))
    # A hoe on dirt: the server tills it and swings the guest's hand, on the guest's screen too.
    # Where the guest stands now (an earlier check can leave it on a ledge it then drops from).
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    soil = (gx, gy - 1, gz + 1)
    guest('command', command=f'setblock {soil[0]} {soil[1]} {soil[2]} minecraft:dirt')
    # HoeItem.useOn tills only under air.
    guest('command', command=f'setblock {soil[0]} {soil[1] + 1} {soil[2]} minecraft:air')
    guest('command', command='give @s minecraft:wooden_hoe')
    guest.wait(10)
    guest.raw('select', 'minecraft:wooden_hoe')
    before = guest('state')['swings']
    guest.raw('interact', *soil)
    guest.wait(10)
    hoe = {'swings': guest('state')['swings'] - before, 'soil': host.raw('block', *soil)['block']}
    guest('command', command=f'setblock {soil[0]} {soil[1]} {soil[2]} minecraft:stone')
    detail = {'host_sees_guest': host_sees, 'guest_sees_host': guest_sees, 'hoe': hoe}
    assert sum(map(bool, host_sees)) >= 4, f'the host does not see the guest mine: {detail}'
    assert sum(map(bool, guest_sees)) >= 4, f'the guest does not see the host mine: {detail}'
    assert hoe == {'swings': 1, 'soil': 'minecraft:farmland'}, f'the guest\'s hoe: {detail}'
    return detail


def check_cracks(host, guest, out):
    # Another player's crack (#1096): ServerLevel.destroyBlockProgress sends each new stage to the
    # others (ClientboundBlockDestructionPacket), drawn by LevelRenderer from destroyingBlocks.
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {gx} {gy} {gz + 1} minecraft:stone')
    host('command', command=f'setblock {hx} {hy} {hz + 1} minecraft:stone')
    host.wait(10)
    shots = []

    def cracks(game, pos, name):
        stage = max((c['stage'] for c in game('status')['destroying'] if tuple(c['pos']) == pos), default=None)
        if stage is not None and stage >= 2 and name not in shots:
            shots.append(name)
            game.shot(out, name)
        return stage

    def watch(miner, watcher, pos, name):
        # The watcher looks at the block the other mines.
        watcher.raw('look-at', f'{pos[0] + 0.5:.2f}', f'{pos[1] + 0.5:.2f}', f'{pos[2] + 0.5:.2f}')
        watcher.wait(5)
        return watch_mining(miner, pos, lambda: cracks(watcher, pos, name))
    host_sees = watch(guest, host, (gx, gy, gz + 1), 'cracks-guest-mines')
    face_each_other(host, guest)
    guest_sees = watch(host, guest, (hx, hy, hz + 1), 'cracks-host-mines')
    host.wait(10)
    # The breaker's -1 (ServerLevel.destroyBlockProgress once the block is broken) clears each crack.
    gone = {name: [(c['pos'], game.raw('block', *c['pos'])['block']) for c in game('status')['destroying']]
            for name, game in (('host', host), ('guest', guest))}
    detail = {'host_sees_guest': host_sees, 'guest_sees_host': guest_sees, 'after': gone}
    stages = lambda s: [x for x in s if x is not None]
    assert len(stages(host_sees)) >= 3 and stages(host_sees) == sorted(stages(host_sees)), \
        f'the host does not see the guest\'s crack grow: {detail}'
    assert len(stages(guest_sees)) >= 3 and stages(guest_sees) == sorted(stages(guest_sees)), \
        f'the guest does not see the host\'s crack grow: {detail}'
    assert not any(gone.values()), f'a crack stays after the block broke: {detail}'
    return detail


def check_eating(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
        # Hungry: Player.canEat.
        g('command', command='effect give @s minecraft:hunger 3 255')
    host.wait(70)
    for g in (host, guest):
        g('command', command='effect clear @s')
        g('command', command='give @s minecraft:bread 2')
    host.wait(5)
    face_each_other(host, guest)

    def using(game):
        # On its server thread the host's window is a client of its server too.
        if game is host and not SERVER_THREAD:
            return lan_guest(host).get('model_using') or 0
        return max((e.get('using') or 0 for e in remote(game).get('player_equipment', [])), default=0)
    seen = {}
    for name, eater, watcher in (('host eats', host, guest), ('guest eats', guest, host)):
        eater.raw('select', 'minecraft:bread')
        hold = threading.Thread(target=eater.raw, args=('use-item', '--hold', 2.2))
        hold.start()
        samples = []
        for _ in range(4):
            time.sleep(0.3)
            samples.append(using(watcher))
        watcher.shot(out, 'eating-' + name.replace(' ', '-'))
        hold.join()
        eater.wait(10)
        seen[name] = {'during': samples, 'after': using(watcher)}
    for name, s in seen.items():
        assert sum(1 for u in s['during'] if u > 0) >= 3 and s['after'] == 0, f'{name}: {seen}'
    return seen


def check_xp(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='xp set @s 0 levels')
        g('command', command='xp set @s 0 points')
    # Out of the orbs' way wherever an earlier check left the host (ExperienceOrb follows the
    # nearest player who isn't a spectator).
    hx, hy, hz = pos_of(host)
    host('command', command='gamemode spectator')
    try:
        guest.wait(10)
        # `xp set` leaves Player.totalExperience (the score): experience from earlier checks stays in it.
        total = guest('state')['player']['xp']['total']
        host_total = host('state')['player']['xp']['total']
        x, y, z = pos_of(guest)
        # Three orbs of 7 points 4 blocks east of the guest (out of reach, within the 8 blocks orbs
        # follow from): they fly to the guest.
        for _ in range(3):
            guest('command', command=f'summon minecraft:experience_orb {x + 4:.2f} {y + 0.5:.2f} {z:.2f} {{Value:7s}}')
        guest.wait(2)
        seen = remote(guest).get('entities', {}).get('minecraft:experience_orb', 0)
        guest.shot(out, 'xp-orbs')
        guest.wait(60)
        xp = guest('state')['player']['xp']
        host_xp = host('state')['player']['xp']
        left = remote(guest).get('entities', {}).get('minecraft:experience_orb', 0)
    finally:
        host('command', command='gamemode survival')
        # Back where it stood: a spectator sinks (Player.travel keeps 0.6 of its fall speed each tick,
        # about 0.2 blocks from standing), and a later check that makes it survival again would land
        # it inside the floor, with the guest teleported beside it (swings).
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
    detail = {'seen': seen, 'guest': xp, 'host': host_xp, 'left': left}
    # ExperienceOrb.scanForMerges: orbs of one value side by side can merge into one.
    assert 1 <= seen <= 3, f'the guest saw {seen} orbs: {detail}'
    assert xp['total'] - total == 21 and xp['level'] == 2, f'the guest\'s experience after three 7-point orbs: {detail}'
    assert host_xp['total'] == host_total, f'the host took the orbs: {detail}'
    assert left == 0, f'orbs left on the guest\'s screen: {detail}'
    return {'seen': seen, 'guest': xp, 'host': host_xp}


def check_time(host, guest, out):
    def times():
        return host('state')['time']['day_time'], guest('state')['time']['day_time']
    host('command', command='time set 1000')
    host('command', command='time pause')
    host.wait(60)
    paused = times()
    host('command', command='time resume')
    guest.wait(20)
    # A stalled game runs at most 10 missed ticks when it wakes (DeltaTracker); the server's
    # game time, sent every second, moves the guest's clock on (ClientClockManager.tick).
    os.kill(guest.pid, signal.SIGSTOP)
    time.sleep(4)
    os.kill(guest.pid, signal.SIGCONT)
    guest.wait(60)
    stalled = times()
    detail = {'paused': paused, 'after_stall': stalled}
    assert paused[0] == paused[1] and 1000 <= paused[0] < 1010, f'the clock while paused: {detail}'
    # The two reads are a tick or two apart.
    assert abs(stalled[0] - stalled[1]) <= 3, f'the guest\'s clock after its game stalled: {detail}'
    return detail


def objects_near(game, centre, key='objects'):
    """`status`'s objects within 8 blocks of centre, without their ids (each side numbers its own)."""
    out = []
    for o in game('status')[key]:
        if all(abs(o['position'][i] - centre[i]) <= 8 for i in (0, 2)):
            o = dict(o)
            o.pop('id')
            o['position'] = [round(v, 2) for v in o['position']]
            if 'yaw' in o:  # as the packets carry it, in 256ths of a turn
                o['yaw'] = round(o['yaw'] * 256 / 360) % 256
            out.append(o)
    return sorted(out, key=json.dumps)


def check_objects(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    commands = [
        f'fill {x - 6} {y} {z - 6} {x + 6} {y + 3} {z - 6} minecraft:stone',
        f'summon minecraft:minecart {x - 4.5} {y} {z + 3.5}',
        f'summon minecraft:chest_minecart {x - 2.5} {y} {z + 3.5}',
        f'summon minecraft:furnace_minecart {x + 1.5} {y} {z + 3.5}',
        f'summon minecraft:tnt_minecart {x + 4.5} {y} {z + 3.5}',
        f'summon minecraft:oak_boat {x - 4.5} {y} {z + 6.5} {{Rotation:[45f,0f]}}',
        f'summon minecraft:birch_chest_boat {x - 1.5} {y} {z + 6.5}',
        f'summon minecraft:bamboo_raft {x + 2.5} {y} {z + 6.5}',
        f'summon minecraft:armor_stand {x - 2.5} {y} {z - 2.5} {{Rotation:[90f,0f]}}',
        f'summon minecraft:armor_stand {x - 4.5} {y} {z - 2.5} {{ShowArms:1b,Small:1b,NoBasePlate:1b,'
        'equipment:{head:{id:"minecraft:diamond_helmet",count:1}}}',
        f'summon minecraft:item_frame {x - 3.5} {y + 1} {z - 4.97} {{block_pos:[I;{x - 4},{y + 1},{z - 5}],Facing:3b,'
        'Item:{id:"minecraft:diamond",count:1},ItemRotation:3b}',
        f'summon minecraft:glow_item_frame {x - 1.5} {y + 1} {z - 4.97} {{block_pos:[I;{x - 2},{y + 1},{z - 5}],Facing:3b}}',
        f'summon minecraft:painting {x + 1.5} {y + 1} {z - 4.97} {{block_pos:[I;{x + 1},{y + 1},{z - 5}],facing:0b,'
        'variant:"minecraft:kebab"}',
        f'summon minecraft:painting {x + 4} {y + 1} {z - 4.97} {{block_pos:[I;{x + 4},{y + 1},{z - 5}],facing:0b,'
        'variant:"minecraft:courbet"}',
    ]
    try:
        for command in commands:
            host('command', command=command)
        host.wait(60)
        guest.raw('look-at', x, y, z - 5)
        guest.wait(10)
        guest.shot(out, 'objects')
        host.shot(out, 'objects')
        mine, theirs = objects_near(host, (x, y, z)), objects_near(guest, (x, y, z))
    finally:
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 8},y={y - 2},z={z - 8},dx=16,dy=8,dz=16]')
        host('command', command=f'fill {x - 6} {y} {z - 6} {x + 6} {y + 3} {z - 6} minecraft:air')
    assert len(mine) == len(commands) - 1, f'the host has {len(mine)} objects: {mine}'
    assert theirs == mine, f'the guest sees {theirs}\nthe host has {mine}'
    return f'{len(theirs)} objects identical'


def check_projectiles(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    commands = [
        f'summon minecraft:tnt {x + 4.5} {y} {z + 4.5} {{fuse:2000s}}',
        f'summon minecraft:end_crystal {x - 4.5} {y} {z + 4.5} {{ShowBottom:0b}}',
        f'summon minecraft:tnt {x + 2.5} {y + 5} {z - 3.5} {{fuse:2000s}}',
        f'summon minecraft:arrow {x - 2.5} {y + 4} {z - 2.5} {{Motion:[0.2d,0d,0d]}}',
        f'summon minecraft:falling_block {x - 0.5} {y + 6} {z - 4.5} {{BlockState:{{Name:"minecraft:sand"}}}}',
    ]
    try:
        for command in commands:
            host('command', command=command)
        host.wait(80)
        guest.raw('look-at', x, y, z)
        guest.wait(5)
        guest.shot(out, 'projectiles')
        mine = objects_near(host, (x, y, z), 'projectiles')
        theirs = objects_near(guest, (x, y, z), 'projectiles')
    finally:
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 8},y={y - 2},z={z - 8},dx=16,dy=10,dz=16]')
        host('command', command=f'setblock {x - 1} {y} {z - 5} minecraft:air')
    assert len(mine) == len(commands) - 1, f'the host has {len(mine)}: {mine}'
    # The fuse counts down every tick: the guest's, read a moment later, may be a tick or two behind.
    fuses = [(m.pop('fuse', 0), t.pop('fuse', 0)) for m, t in zip(mine, theirs)]
    assert theirs == mine, f'the guest sees {theirs}\nthe host has {mine}'
    assert all(abs(a - b) <= 2 for a, b in fuses), f'fuses (host, guest): {fuses}'
    return f'{len(theirs)} identical, fuses (host, guest) {fuses}'


def check_blast(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    host('command', command='gamemode creative')
    x, y, z = (int(v // 1) for v in pos_of(host))
    gx, gz = x + 6, z
    floor = f'fill {gx - 6} {y - 1} {gz - 4} {gx + 6} {y - 1} {gz + 4} minecraft:bedrock'
    detail = {}
    try:
        for shielded in (False, True):
            if remote(guest)['health'] <= 0:
                guest.raw('respawn')
                guest.wait(40)
            host('command', command=floor)
            host('command', command=f'fill {gx - 6} {y} {gz - 4} {gx + 6} {y + 3} {gz + 4} minecraft:air')
            guest('command', command=f'tp @s {gx + 0.5} {y} {gz + 0.5}')
            guest('command', command='effect give @s minecraft:instant_health 1 5')
            guest.wait(30)
            hold = None
            if shielded:
                guest('command', command='item replace entity @s weapon.offhand with minecraft:shield')
                guest.wait(5)
                guest.raw('look-at', gx + 5.5, y + 0.5, gz + 0.5)
                hold = threading.Thread(target=guest.raw, args=('use-item', '--hold', 2.5))
                hold.start()
                time.sleep(0.5)
            before = remote(guest)['health']
            at = pos_of(guest)
            # Five blocks off: a blast that hurts without killing (about 14 at full exposure).
            host('command', command=f'summon minecraft:tnt {gx + 5.5} {y} {gz + 0.5} {{fuse:10s}}')
            # The guest's own controller is busy holding the shield up: the host waits.
            host.wait(14)
            after = remote(guest)['health']
            moved = pos_of(guest)
            if hold:
                hold.join()
            guest.shot(out, f'blast-{"shield" if shielded else "open"}')
            detail['shield' if shielded else 'open'] = {
                'before': before, 'after': after, 'pushed': round(at[0] - moved[0], 2)}
    finally:
        host('command', command='gamemode survival')
        guest('command', command='clear @s')
        host('command', command=floor)
    open_, shield = detail['open'], detail['shield']
    assert open_['after'] < open_['before'], f'the blast never hurt the guest: {detail}'
    assert open_['pushed'] > 0, f'the blast never pushed the guest away: {detail}'
    assert shield['after'] >= shield['before'], f'the shield did not stop the blast: {detail}'
    return detail


def light_differences(host, guest, lo, hi):
    """Positions of the box loaded on both sides, and those whose sky or block light differ."""
    h, g = host.raw('light', *lo, *hi), guest.raw('light', *lo, *hi)
    both = [i for i in range(len(h['sky'])) if h['sky'][i] != '.' and g['sky'][i] != '.']
    differ = [i for i in both if (h['sky'][i], h['block'][i]) != (g['sky'][i], g['block'][i])]
    return h, g, both, differ


def same_light(host, guest, lo, hi, what, ticks=100):
    """Wait until the guest's light over the box is the host's; the number of positions compared."""
    for _ in range(ticks // 10):
        h, g, both, differ = light_differences(host, guest, lo, hi)
        if not differ:
            assert both, f'{what}: no position of the box is loaded on both sides'
            return len(both)
        host.wait(10)
    i = differ[0]
    nx, nz = hi[0] - lo[0] + 1, hi[2] - lo[2] + 1
    pos = (lo[0] + i % nx, lo[1] + i // (nx * nz), lo[2] + i // nx % nz)
    raise AssertionError(f'{what}: {len(differ)} of {len(both)} positions differ, first {pos}: host sky '
                         f'{h["sky"][i]} block {h["block"][i]}, guest sky {g["sky"][i]} block {g["block"][i]}')


def check_flight(host, guest, out):
    def double_jump(game):
        # LocalPlayer.aiStep: a second jump press within 7 ticks of the first toggles flight.
        for hold in (1, 4):
            game.raw('key', 'space', 'down')
            game.wait(hold)
            game.raw('key', 'space', 'up')
            game.wait(2)
        game.wait(10)
        return game('state')['player']

    def set_mode(game, mode):
        # On a client of a server the mode and its abilities (may fly) come back with the server's
        # answer, later when the machine is busy: a double jump before them is two plain jumps.
        game('command', command=f'gamemode {mode}')
        for _ in range(100):
            if game('state').get('gamemode') == mode:
                break
            game.wait(1)
        game.wait(10)

    players = [('guest', guest)] + ([('host', host)] if SERVER_THREAD else [])
    seen = {}
    try:
        for name, game in players:
            set_mode(game, 'creative')
            ground = game('state')['player']
            up = double_jump(game)
            game.wait(20)
            later = game('state')['player']
            set_mode(game, 'survival')
            fell = game('state')['player']
            seen[name] = {'flying': up['flying'], 'rose': round(up['position'][1] - ground['position'][1], 2),
                          'stayed': round(later['position'][1] - up['position'][1], 2),
                          'survival_flying': fell['flying']}
    finally:
        for _, game in players:
            game('command', command='gamemode survival')
    for name, s in seen.items():
        assert s['flying'] and s['rose'] > 1 and abs(s['stayed']) < 0.1, f'{name} does not fly in creative: {seen}'
        assert not s['survival_flying'], f'{name} still flies in survival: {seen}'
    return seen


def check_growth(host, guest, out):
    """Block ticks near a guest 1000 blocks from the host: only the guest's tickets tick them."""
    x0, y0, z0 = pos_of(guest)
    guest('command', command='gamemode creative')
    guest('command', command=f'tp @s {x0 + 1000:.2f} 200 {z0:.2f}')
    guest.wait(100)
    x, y, z = (int(v // 1) for v in pos_of(guest))
    # A platform in the air over wherever the guest landed: a water source two blocks east of the
    # guest, wheat on farmland two blocks west. Vanilla's water spreads only towards the nearest
    # drop (FlowingFluid.getSpread), so any of the source's four neighbours may take it. Wheat
    # grows only in daylight (raw brightness 9 or more).
    top = max(y + 12, 120)
    water = (x + 2, top + 1, z)
    beside = [(water[0] + dx, water[1], water[2] + dz) for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1))]
    wheat = (x - 2, top + 1, z)
    flowed = []
    try:
        host('command', command='time set noon')
        guest('command', command=f'fill {x - 4} {top} {z - 2} {x + 4} {top} {z + 2} minecraft:stone')
        guest('command', command=f'setblock {wheat[0]} {top} {wheat[2]} minecraft:farmland[moisture=7]')
        guest('command', command=f'setblock {wheat[0]} {wheat[1]} {wheat[2]} minecraft:wheat[age=0]')
        guest('command', command='gamerule random_tick_speed 1000')
        guest('command', command=f'setblock {water[0]} {water[1]} {water[2]} minecraft:water')
        for _ in range(40):
            guest.wait(10)
            flowed = [p for p in beside if guest.raw('block', *p).get('block') == 'minecraft:water']
            if flowed:
                break
        age = wait_state(guest, wheat, 'age', '7', 2400)
        guest.shot(out, 'growth')
    finally:
        guest('command', command='gamerule random_tick_speed 3')
        host('command', command='time set day')
        guest('command', command=f'tp @s {x0:.2f} {y0:.2f} {z0:.2f}')
        guest('command', command='gamemode survival')
        guest.wait(40)
    detail = {'flowed': flowed, 'wheat age': age}
    assert flowed, f'the water beside the guest did not flow: {detail}'
    assert age == '7', f'the wheat beside the guest did not grow: {detail}'
    return detail


def check_light(host, guest, out):
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    results = {}
    # A torch near the guest: the guest lights it itself, as vanilla's client does.
    torch = (gx + 3, gy, gz + 3)
    host('command', command=f'setblock {torch[0]} {torch[1]} {torch[2]} minecraft:torch')
    try:
        results['torch'] = same_light(host, guest, (gx - 12, gy - 1, gz - 12), (gx + 16, gy + 3, gz + 16), 'torch')
    finally:
        host('command', command=f'setblock {torch[0]} {torch[1]} {torch[2]} minecraft:air')
    # A roof over the guest, then a hole in it: sky light falls through.
    roof = f'{gx - 3} {gy + 4} {gz - 3} {gx + 3} {gy + 4} {gz + 3}'
    box = (gx - 6, gy - 1, gz - 6), (gx + 6, gy + 5, gz + 6)
    host('command', command=f'fill {roof} minecraft:stone')
    try:
        results['roof'] = same_light(host, guest, *box, 'roof')
        host('command', command=f'setblock {gx} {gy + 4} {gz} minecraft:air')
        results['hole'] = same_light(host, guest, *box, 'hole in the roof')
    finally:
        host('command', command=f'fill {roof} minecraft:air')
    # The last chunk the guest tracks eastwards, and glowstone high up just past it in a chunk it
    # doesn't track: the guest never sees that block, only the light updates of its border chunk.
    y = 200
    row = guest.raw('light', gx, y, gz, gx + 16 * 24, y, gz)['sky']
    assert '.' in row, 'the guest has every chunk 24 chunks east'
    untracked = (gx + row.index('.')) >> 4
    near = untracked * 16 + 1
    # On its server thread the host's own game is a client too, sent the same light updates.
    if not SERVER_THREAD:
        assert host.raw('light', near, y, gz, near, y, gz)['sky'] != '.', 'the host lacks the chunk past the guest'
    host('command', command=f'setblock {near} {y} {gz} minecraft:glowstone')
    try:
        lo, hi = (untracked * 16 - 16, y - 3, gz - 8), (untracked * 16 - 1, y + 3, gz + 8)
        # On the server thread the host's game is a client too, as dark as the guest until the light
        # update arrives: wait for it before comparing them.
        for _ in range(10):
            lit = guest.raw('light', near - 2, y, gz, near - 2, y, gz)['block']
            if lit == 'd':
                break
            host.wait(10)
        assert lit == 'd', f'the border chunk by the glowstone has block light {lit}, not 13, after 100 ticks'
        results['border'] = same_light(host, guest, lo, hi, 'border chunk')
    finally:
        host('command', command=f'setblock {near} {y} {gz} minecraft:air')
    results['border_cleared'] = same_light(host, guest, lo, hi, 'border chunk without the glowstone')
    return results


def check_fireball(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    fx, fz = x + 4, z
    # A stone floor, a dispenser facing down three blocks over it, a fire charge in it, powered.
    host('command', command=f'fill {fx - 2} {y - 1} {fz - 2} {fx + 2} {y - 1} {fz + 2} minecraft:stone')
    host('command', command=f'fill {fx - 2} {y} {fz - 2} {fx + 2} {y + 4} {fz + 2} minecraft:air')
    host('command', command=f'setblock {fx} {y + 3} {fz} minecraft:dispenser[facing=down]')
    host('command', command=f'item replace block {fx} {y + 3} {fz} container.0 with minecraft:fire_charge')
    seen, gone = {}, None
    try:
        host('command', command=f'setblock {fx} {y + 4} {fz} minecraft:redstone_block')
        host.wait(10)
        seen = {'host': host.raw('block', fx, y, fz).get('block'),
                'guest': guest.raw('block', fx, y, fz).get('block')}
        # FireBlock.tick every 30-39 ticks ages it by one a third of the time; past age 3 it goes.
        for waited in range(40, 1240, 40):
            host.wait(40)
            if host.raw('block', fx, y, fz).get('block') == 'minecraft:air':
                gone = waited
                break
        guest.wait(5)
        seen['guest_after'] = guest.raw('block', fx, y, fz).get('block')
    finally:
        host('command', command=f'fill {fx - 2} {y} {fz - 2} {fx + 2} {y + 4} {fz + 2} minecraft:air')
    assert seen.get('host') == 'minecraft:fire' and seen.get('guest') == 'minecraft:fire', \
        f'no fire where the fire charge landed: {seen}'
    assert gone is not None, f'the fire on stone never went out (1200 ticks): {seen}'
    assert seen['guest_after'] == 'minecraft:air', f'the guest still sees the fire: {seen}'
    return {'out_after_ticks': gone, **seen}


def check_deflect(host, guest, out):
    """The host punches a ghast's fireball back: it flies along the host's view and kills the ghast
    (Player.attack: AIM_DEFLECT on #redirectable_projectile; Ghast.isReflectedFireball). The ghast is caged
    (a window toward the host); the game runs at 4 ticks a second, and once the fireball is within 4.5 blocks
    the host looks at the ghast and clicks."""
    if SERVER_THREAD:
        # The fireball's position and the host's hurts are read from the host's own level.
        return 'skipped (in-frame only)'
    x, y, z = (int(v // 1) for v in pos_of(host))
    guest('command', command='gamemode creative')
    host('command', command='gamemode survival')
    host('command', command='difficulty normal')
    gx, gz = x, z + 30
    gy = y + 20
    host('command', command=f'fill {gx - 3} {gy - 1} {gz - 3} {gx + 24} {gy - 1} {gz + 3} minecraft:glass')
    host('command', command=f'fill {gx - 3} {gy} {gz - 3} {gx + 24} {gy + 8} {gz + 3} minecraft:air')
    host('command', command=f'tp @s {gx + 0.5} {gy} {gz + 0.5} -90 0')
    host.wait(20)
    host.raw('hurtlog')
    # The ghast in a glass cage it can't drift out of, a 3 x 3 window toward the host.
    host('command', command=f'fill {gx + 13} {gy} {gz - 3} {gx + 19} {gy + 6} {gz + 3} minecraft:glass hollow')
    host('command', command=f'fill {gx + 13} {gy + 2} {gz - 1} {gx + 13} {gy + 4} {gz + 1} minecraft:air')
    host('command', command=f'summon minecraft:ghast {gx + 16.5} {gy + 1} {gz + 0.5}')
    eye = (gx + 0.5, gy + 1.62, gz + 0.5)
    host('command', command='tick rate 4')
    turned, near, ghast, hurt = None, None, None, []
    try:
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            balls = [q for q in host.raw('status')['projectiles'] if q['kind'] == 'LargeFireball']
            near = min((math.dist(q['position'], eye) for q in balls), default=None)
            if near is not None and near < 4.5:
                g = [m['pos'] for m in host.raw('mobs', 60).get('near', []) if m['kind'].endswith('ghast')][0]
                host.raw('look-at', f'{g[0]:.2f}', f'{g[1] + 2:.2f}', f'{g[2]:.2f}')
                host.raw('mouse', 'left', 'down')
                host.raw('mouse', 'left', 'up')
                turned = near
                break
        host('command', command='tick rate 20')
        host.wait(60)
        ghast = [m for m in host.raw('mobs', 60).get('near', []) if m['kind'].endswith('ghast') and m['health'] > 0]
        hurt = [h for pl in host.raw('hurtlog')['players'] if pl['name'] == 'host' for h in pl['hurts']
                if 'fireball' in h['source'] or 'explosion' in h['source']]
    finally:
        host('command', command='tick rate 20')
        host('command', command='kill @e[type=minecraft:ghast]')
        host('command', command=f'fill {gx - 3} {gy - 1} {gz - 3} {gx + 24} {gy + 8} {gz + 3} minecraft:air')
        guest('command', command='gamemode survival')
    assert turned is not None, f'no fireball came within 4.5 blocks (nearest {near})'
    assert not hurt, f'the host was hit: {hurt}'
    assert not ghast, f'the ghast lived: the fireball was not turned back ({ghast})'
    return {'struck_at': round(turned, 2)}


def check_barter(host, guest, out):
    """The guest, in gold boots, drops a gold ingot to a piglin: the piglin stays calm, takes it, and throws its
    barter toward the nearest player, the guest (PiglinAi.throwItemsTowardPlayer: NEAREST_VISIBLE_PLAYER), who
    picks it up. The host is nearby on the other side, so a barter thrown at the host misses the guest. A glass
    corridor (the hollow box) keeps the piglin between them."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    cz = z + 4
    host('command', command='gamemode creative')
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='effect give @s minecraft:instant_health 1 5')
    host('command', command=f'fill {x + 1} {y - 1} {cz - 1} {x + 8} {y + 2} {cz + 1} minecraft:glass hollow')
    host('command', command=f'fill {x + 2} {y - 1} {cz} {x + 7} {y - 1} {cz} minecraft:stone')
    host('command', command=f'tp @s {x - 0.5} {y} {cz + 0.5} -90 0')
    guest('command', command=f'tp @s {x + 7.3} {y} {cz + 0.5} 90 30')
    guest('command', command='item replace entity @s armor.feet with minecraft:golden_boots')
    guest('command', command='give @s minecraft:gold_ingot 1')
    host('command', command=f'summon minecraft:piglin {x + 4.5} {y} {cz + 0.5} {{IsImmuneToZombification:1b}}')
    guest.wait(20)
    before = remote(guest)['health']
    kept = dict(guest('inventory')['counts'])
    try:
        guest.raw('select', 'minecraft:gold_ingot')
        guest.raw('key', 'q', 'tap')
        bought, trace = {}, []
        for _ in range(20):
            guest.wait(20)
            trace.append([(e['type'].split(':')[-1], round(e['pos'][0] - x, 1), round(e['pos'][2] - cz, 1))
                          for e in host.raw('entities', 24)['entities'] if e['type'] in ('minecraft:item', 'minecraft:piglin')])
            counts = guest('inventory')['counts']
            bought = {k: n - kept.get(k, 0) for k, n in counts.items()
                      if k not in ('minecraft:gold_ingot', 'minecraft:golden_boots') and n > kept.get(k, 0)}
            if bought:
                guest.wait(40)
                break
        counts = guest('inventory')['counts']
        bought = {k: n - kept.get(k, 0) for k, n in counts.items()
                  if k not in ('minecraft:gold_ingot', 'minecraft:golden_boots') and n > kept.get(k, 0)}
        piglins = [m for m in host.raw('mobs', 20).get('near', []) if m['kind'].split(':')[-1] == 'piglin']
        health = remote(guest)['health']
    finally:
        host('command', command=f'kill @e[type=minecraft:piglin,x={x},y={y - 2},z={cz - 2},dx=10,dy=6,dz=4]')
        # What lies on the floor, on the host's side of the piglin's start and on the guest's.
        floor = [host('command', command=f'kill @e[type=minecraft:item,x={x0},y={y - 2},z={cz - 3},dx={dx},dy=6,dz=6]')
                 ['chat'][0] for x0, dx in ((x - 2, 6.5), (x + 4.5, 6))]
        host('command', command=f'fill {x + 1} {y - 1} {cz - 1} {x + 8} {y + 2} {cz + 1} minecraft:air')
        host('command', command=f'fill {x - 8} {y - 1} {z - 8} {x + 8} {y - 1} {z + 8} minecraft:stone')
        guest('command', command='clear @s')
        host('command', command='gamemode survival')
    assert piglins, 'the piglin is gone'
    assert health >= before, f'the piglin hurt the guest in gold boots: {before} -> {health}'
    assert bought, f'no barter reached the guest (on the floor, host side then guest side: {floor}; items and the piglin, every second: {trace})'
    return {'bought': bought, 'floor': floor}


def check_worn(host, guest, out):
    pieces = ['iron_helmet', 'iron_chestplate', 'iron_leggings', 'iron_boots']
    guest('command', command='gamemode survival')
    taken = {}
    try:
        for worn in (False, True):
            guest('command', command='clear @s')
            if remote(guest)['health'] <= 0:
                guest.raw('respawn')
                guest.wait(40)
            if worn:
                for piece in pieces:
                    guest('command', command=f'give @s minecraft:{piece}')
                guest.wait(10)
                # Shift-clicked from the hotbar onto the body in the inventory menu (the hub moves them).
                guest.raw('key', 'e', 'tap')
                guest.raw('wait-for', 'screen', 'inventory')
                for piece in pieces:
                    guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{piece}'), 'shift')
                    guest.wait(2)
                guest.raw('key', 'esc', 'tap')
                guest.wait(5)
            guest('command', command='effect give @s minecraft:instant_health 1 5')
            guest.wait(30)
            before = remote(guest)['health']
            if not SERVER_THREAD:
                host.raw('hurtlog')
            # A mob's blow (no attacker: no difficulty scaling); armour counts against it.
            guest('command', command='damage @s 10 minecraft:mob_attack')
            if SERVER_THREAD:
                # The server's hurt log isn't on a control socket here: the health the guest is
                # sent, read as soon as it drops (natural regeneration heals it again after).
                health = before
                for _ in range(40):
                    guest.wait(1)
                    health = remote(guest)['health']
                    if health < before:
                        break
                taken['worn' if worn else 'bare'] = [round(before - health, 2)]
                continue
            guest.wait(10)
            log = [h for p in host.raw('hurtlog')['players'] if p['name'] == GUEST_NAME for h in p['hurts']]
            taken['worn' if worn else 'bare'] = [(h['amount'], h['armor'], round(h['taken'], 2)) for h in log]
    finally:
        guest('command', command='clear @s')
        guest('command', command='effect give @s minecraft:instant_health 1 5')
    if SERVER_THREAD:
        # Natural regeneration may heal up to a point before the health arrives.
        assert len(taken['bare']) == 1 and 9.0 <= taken['bare'][0] <= 10.0, f'10 damage on no armour: {taken}'
        assert len(taken['worn']) == 1 and 5.0 <= taken['worn'][0] <= 6.0, \
            f'10 damage through iron armour (15) should take 6: {taken}'
        return taken
    assert taken['bare'] == [(10.0, 0.0, 10.0)], f'10 damage on no armour: {taken}'
    assert taken['worn'] == [(10.0, 15.0, 6.0)], f'10 damage through iron armour (15) should take 6: {taken}'
    return taken


CHECK_FNS = {'tab': check_tab, 'self': check_self, 'armour': check_armour, 'nametags': check_nametags,
             'chest': check_chest, 'crafting': check_crafting, 'sign': check_sign, 'damage': check_damage,
             'mobs': check_mobs, 'hazards': check_hazards, 'pvp': check_pvp,
             'respawn': check_respawn,
             'containers': check_containers, 'feedback': check_feedback, 'titles': check_titles, 'chat': check_chat,
             'machines': check_machines,
             'brewing': check_brewing,
             'hurt': check_hurt,
             'commands': check_commands,
             'recipes': check_recipes, 'trials': check_trials, 'walls': check_walls,
             'effects': check_effects, 'portals': check_portals, 'distance': check_distance, 'clicks': check_clicks,
             'sleep': check_sleep, 'push': check_push, 'eat': check_eat, 'eating': check_eating,
             'xp': check_xp, 'objects': check_objects, 'projectiles': check_projectiles, 'blast': check_blast,
             'light': check_light, 'plates': check_plates,
             'swings': check_swings, 'cracks': check_cracks,
             'time': check_time, 'growth': check_growth, 'flight': check_flight, 'fireball': check_fireball, 'deflect': check_deflect, 'barter': check_barter, 'worn': check_worn, 'kills': check_kills, 'arrows': check_arrows, 'predict': check_predict}


def run(ctl, out, resources, checks):
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    # Either way explicitly: the game's default is the server thread.
    host.d.env['HORIZON_SERVER_THREAD'] = '1' if SERVER_THREAD else '0'
    results = {}
    try:
        host.pid = host('launch', wait=False, offscreen=sys.platform == 'darwin')['pid']
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
        guest.pid = guest('launch', wait=False, offscreen=sys.platform == 'darwin')['pid']
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
    parser.add_argument('--server-thread', action='store_true', help="the host's world runs on its server thread")
    args = parser.parse_args()
    global SERVER_THREAD
    SERVER_THREAD = args.server_thread
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--bins'],
                       cwd=ROOT, check=True)
        meta = json.loads(subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT,
                                         capture_output=True, text=True, check=True).stdout)
        bins = Path(meta['target_directory']) / 'fast'
    resources = lab.RESOURCES.resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    results = run(bins / 'horizon-ctl', out, resources, args.check or CHECKS)
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
