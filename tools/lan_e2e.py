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
- looks:     mobs look on both sides as the server has them (their synched data): a calf is a
             baby, a red sheep is sheared, a size 4 slime, a tamed wolf sits with its blue collar, a
             cow on fire burns and goes out; then the wither's health drops when it is hurt, the
             sheep is dyed blue with its wool back and the calf grows up (#1367, #1368, #1370,
             #1371, #1385, #1390);
- leads:     the host and the guest each tie a cow to a lead and untie it: the rope shows on both
             sides, held by the player who tied it, and goes on both (#1360);
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
- loom:      a loom opens on the guest's screen; the banner and dye the guest shift-clicks go to their
             slots, a pattern button the guest clicks makes the server's result, and the guest takes the
             patterned banner (#868);
- stonecutter: a stonecutter opens on the guest's screen with the recipes of the stone the guest puts
             in; the guest picks the slab and shift-clicks the result, and the server has cut every
             stone into slabs (#868);
- grindstone: a grindstone opens on the guest's screen; the enchanted sword the guest shift-clicks in
             shows the server's result, and the guest takes it with only its curse left and the
             experience orbs the server awards (#868);
- anvil:     an anvil opens on the guest's screen; the guest repairs a sword with iron ingots and
             renames it (ServerboundRenameItemPacket), takes it for the server's level cost, and the
             server has the repaired, renamed sword (#868);
- enchanting: an enchanting table opens on the guest's screen with the server's offers for the
             sword the guest puts in; the guest clicks the first offer and the server enchants the
             sword for a level and a lapis, and counts it in the guest's enchant_item stat (#868);
- smithing:  a smithing table opens on the guest's screen; the guest upgrades a diamond sword to
             netherite and the server has it (#868);
- lectern:   a lectern's written book opens on the guest's screen; the guest turns its pages
             (ContainerButtonClick) and the server's lectern follows, then the guest takes the book
             and the server has it in the guest's inventory (#868);
- cartography: a cartography table opens on the guest's screen; the guest puts in a map the host
             made and paper, takes the zoomed-out map, and the server has a new map for it (#868);
- feedback:  the guest's command feedback names the guest and shows the numbers;
- chat:      a line the host sends reaches the guest's event log (`wait-for chat`), and on the
             server thread the host window's too;
- effects:   a beacon's power reaches every player in range, the host and the guest;
- distance:  (--server-thread) the server follows the host's render distance, past vanilla's 32 too
             (IntegratedServer.tickServer), and the guest's chunks follow the server's (#804)
- portals:   (--server-thread) the server holds a level per dimension: the guest goes to the Nether
             through a portal while the host stays, then the host goes to the End while the guest is in
             the Nether, and both come back.
- levels:    (--server-thread) the host and the guest each in a level of their own: `execute in`
             teleports into another level, and `execute in` and `at` change and test blocks there;
             mobs fall in two levels at once, with ids unique across them; the guest rejoins in the
             Nether, dies there and respawns in the Overworld, then in the Nether at a `spawnpoint`
             and at a respawn anchor set there; the host takes a Nether portal alone;
             the guest takes an End portal, an End gateway and the exit portal alone; and the host
             saves, quits and reopens with the guest in the Nether.
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
- thrown:    a splash potion and an eye of ender the host and the guest each throw fly on both
             sides (#1363, #1391);
- sparks:    level events' particles on both sides: the host's and the guest's bone meal on wheat
             (growth sparks), honeycomb on copper and an axe taking the wax off, and a spawner's
             burst when it spawns (#1395, #1401, #1402);
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
- ladder:    the guest climbs down ten blocks of ladder and lands unhurt: the server resets its fall
             distance on a ladder every tick, as LivingEntity.handleOnClimbable does (#1255).
- boats:     the host (single player, on its server thread or not) and the guest each right-click a
             boat on water and ride it, paddle it forward (the other side sees it move), sneak out,
             and break it with their fists (#1222).
- minecarts: the host and the guest each get into a minecart on rails, sneak out beside it, and break
             it (#1222).
- frames:    the host and the guest each put a diamond in an item frame and turn it, and put a
             helmet on an armor stand and take it back (#1222).
- mounts:    the host and the guest each open a tamed horse's, donkey's and llama's inventory
             (shift + right click, then the inventory key while riding), put on a saddle, armour
             or a carpet and fill the chest by shift-clicking.
- riding:    the host and the guest each saddle and ride a horse (walk, jump), a pig and a strider
             (with their sticks), a camel and a donkey, sit on a llama and fly a happy ghast, and
             get off by sneaking; the other side sees the animal move.
- lightning: a bolt summoned beside the host, then the guest: both sides get the bolt and flash the
             sky (on its server thread the host's window is a client of it too).
- drops:     an item lies where the host, then the guest, places a block: it comes out of the block
             on the server, and both sides see it outside (#1252).
- underwater: the host and the guest each dig dirt by hand standing under water: five times slower
             than in the air on their own client as on the server, and it breaks (#1254).
- worldspawn: the host moves the world spawn (`setworldspawn`): the host and the guest, with no bed,
             respawn there (#1310).
- shield:    a vindicator's axe disables the host's, then the guest's, raised shield: the server
             counts its 5 s cooldown down once a tick (#1316).
- beds:      the host and the guest each set their respawn point at a bed and die elsewhere: they
             stand up beside the bed, or, when the bed is gone or boxed in by stone, come back at
             the world spawn (#1308).
- deathmining: the host and the guest each die while breaking obsidian and let go on the death
             screen: once respawned, the obsidian stays (#1315).
- contacts:  the host and the guest each stand on magma (and sneak on it), stand on a lit campfire,
             walk against a cactus and through grown sweet berry bushes: the server hurts them, but
             not the sneaking player or young bushes (#1305, #1311, #1312, #1313).
- absorption: the host and the guest each take Absorption II and a hit of 3: their own HUD has the
             server's golden hearts, 8 then 5 (#1306).
- breath:    the host and the guest each stay underwater for 15 s in a Respiration III helmet: their
             own bubbles follow the server's air, which the enchantment makes last (#1304).
- lava:      (--server-thread) the host and the guest each stand in a Nether lava current with Fire
             Resistance, which pushes them at the Nether's strength (#1309).
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
CHECKS = ['tab', 'self', 'armour', 'nametags', 'walls', 'chest', 'crafting', 'sign', 'damage', 'mobs', 'hazards', 'pvp', 'arrows', 'kills', 'hurt', 'looks', 'leads', 'respawn', 'containers', 'feedback', 'titles', 'chat', 'commands', 'recipes', 'machines', 'brewing', 'loom', 'stonecutter', 'grindstone', 'anvil', 'smithing', 'enchanting', 'beacon', 'crafter', 'lectern', 'cartography', 'trials', 'clicks', 'sleep', 'predict', 'push', 'eat', 'eating', 'xp', 'objects', 'projectiles', 'thrown', 'sparks', 'hud', 'sidebar', 'worn', 'blast', 'barter', 'deflect', 'fireball', 'light', 'time', 'swings', 'cracks', 'plates', 'effects', 'portals', 'distance', 'growth', 'flight', 'ladder', 'boats', 'minecarts', 'frames', 'lightning', 'riding', 'mounts', 'worldspawn', 'beds', 'shield', 'deathmining', 'contacts', 'absorption', 'breath', 'lava', 'levels', 'drops', 'underwater']


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
    # On a server the guest is held where it entered until the chunks around the destination are
    # lit, then placed in its portal: wait for that before reading where it arrived.
    for _ in range(30):
        guest.wait(20)
        arrived = [int(v // 1) for v in pos_of(guest)]
        if guest.raw('block', *arrived).get('block') == 'minecraft:nether_portal':
            break
    guest.shot(out, 'portals-nether')
    host_dim = host('state').get('dimension')
    assert there.get('dimension') == 'minecraft:the_nether', f'the guest is in {there.get("dimension")}'
    assert host_dim == 'minecraft:overworld', f'the host went to {host_dim} with the guest'
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


NETHER, END, OVERWORLD = 'minecraft:the_nether', 'minecraft:the_end', 'minecraft:overworld'


def check_levels(host, guest, out):
    """(--server-thread) Each player in its own level (MinecraftServer.levels), the host and the guest
    apart: the steps run in order, each reported, and the check fails if any did."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    start = {g.name: pos_of(g) for g in (host, guest)}
    steps, failed = {}, []
    guest('command', command='gamemode creative')
    try:
        for name, step in LEVEL_STEPS:
            try:
                steps[name] = step(host, guest, out)
            except (Failed, AssertionError, StopIteration, KeyError) as e:
                steps[name], failed = repr(e), failed + [name]
                for g in (host, guest):
                    back_to_game(g)
            print(f'[lan] levels {name}: {"FAIL" if name in failed else "PASS"} {steps[name]}')
    finally:
        for g in (host, guest):
            back_to_game(g)
            if g('state').get('screen') == 'dead':
                g.raw('respawn')
                g.wait(20)
            x, y, z = start[g.name]
            g('command', command=f'execute in {OVERWORLD} run tp @s {x:.2f} {y:.2f} {z:.2f}')
        guest('command', command='gamemode survival')
        guest.wait(40)
    assert not failed, f'failed: {failed}; {steps}'
    return steps


def level_floor(game, x, y, z):
    """An obsidian floor under x y z with air above it, in the level `game` is in (fill needs the
    chunks loaded: ServerLevel.hasChunksAt)."""
    game('command', command=f'fill {x - 4} {y - 1} {z - 4} {x + 4} {y - 1} {z + 4} minecraft:obsidian')
    game('command', command=f'fill {x - 4} {y} {z - 4} {x + 4} {y + 4} {z + 4} minecraft:air')


def into_level(game, dimension, x, y, z):
    """`execute in DIMENSION run tp` to x y z, onto a floor built once the level has loaded around it."""
    game('command', command=f'execute in {dimension} run tp @s {x + 0.5} {y} {z + 0.5}')
    state = wait_dimension(game, dimension, 400)
    game.wait(20)
    level_floor(game, x, y, z)
    game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    game.wait(20)
    return state


def dims(host, guest):
    return host('state').get('dimension'), guest('state').get('dimension')


def level_execute(host, guest, out):
    """`execute in the_nether run tp` takes the guest alone. From the Nether, the guest's commands
    reach the host's level (CommandSourceStack.getLevel): `execute in overworld` and `execute at`
    the host put a block beside the host, and `execute if block` reads it."""
    x, z = (int(v // 1) for v in pos_of(guest)[::2])
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    there = into_level(guest, NETHER, x, 100, z)
    below = guest.raw('block', x, 99, z)
    gold = f'{hx} {hy} {hz + 2} minecraft:gold_block'
    try:
        guest('command', command=f'execute in {OVERWORLD} run setblock {gold}')
        guest('command', command=f'execute at {host.player} run setblock ~ ~ ~-2 minecraft:diamond_block')
        guest('command', command=f'execute in {OVERWORLD} if block {gold} in {NETHER} '
                                  f'run setblock {x} 99 {z} minecraft:gold_block')
        host.wait(10)
        placed = [host.raw('block', hx, hy, hz + 2), host.raw('block', hx, hy, hz - 2)]
        found = guest.raw('block', x, 99, z)
    finally:
        host('command', command=f'fill {hx} {hy} {hz - 2} {hx} {hy} {hz + 2} minecraft:air')
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert there.get('dimension') == NETHER and below.get('block') == 'minecraft:obsidian', f'{there} on {below}'
    assert [p.get('block') for p in placed] == ['minecraft:gold_block', 'minecraft:diamond_block'], \
        f'beside the host: {placed}'
    assert found.get('block') == 'minecraft:gold_block', f'execute in overworld if block: {found}'
    return {'guest': pos_of(guest)}


def level_ticking(host, guest, out):
    """Mobs and objects fall in both levels at once (each level with a player ticks); every entity
    has its own id across the levels."""
    gx, gy, gz = pos_of(guest)
    hx, hy, hz = pos_of(host)
    # A floor of its own beside the host (earlier checks may have broken the open one there).
    bx, by, bz = int(hx // 1), int(hy // 1), int(hz // 1)
    host('command', command=f'fill {bx - 4} {by - 1} {bz - 4} {bx + 4} {by - 1} {bz - 2} minecraft:stone')
    host('command', command=f'fill {bx - 4} {by} {bz - 4} {bx + 4} {by + 4} {bz - 2} minecraft:air')
    guest('command', command=f'summon minecraft:pig {gx + 2.5:.2f} {gy + 3:.2f} {gz:.2f}')
    guest('command', command=f'summon minecraft:armor_stand {gx - 2.5:.2f} {gy + 3:.2f} {gz:.2f}')
    host('command', command=f'summon minecraft:pig {hx + 2.5:.2f} {hy + 3:.2f} {hz - 3:.2f}')
    host('command', command=f'summon minecraft:armor_stand {hx - 2.5:.2f} {hy + 3:.2f} {hz - 3:.2f}')
    try:
        guest.wait(60)
        seen = {}
        for name, g, floor in (('guest', guest, gy), ('host', host, hy)):
            mobs = [m for m in g.raw('mobs', 8)['near'] if m['kind'] == 'minecraft:pig']
            stands = [o for o in g('status')['objects'] if o['kind'] == 'minecraft:armor_stand']
            seen[name] = {'pigs': [(m['id'], round(m['pos'][1], 2)) for m in mobs],
                          'stands': [(o['id'], round(o['position'][1], 2)) for o in stands], 'floor': floor}
    finally:
        guest('command', command='kill @e[type=!minecraft:player,distance=..8]')
        host('command', command='kill @e[type=!minecraft:player,distance=..8]')
    for name, s in seen.items():
        heights = [y for _, y in s['pigs'] + s['stands']]
        assert len(s['pigs']) == 1 and len(s['stands']) == 1, f'{name} sees {s}'
        assert all(abs(y - s['floor']) < 0.1 for y in heights), f"{name}'s did not fall to the floor: {seen}"
    ids = [i for s in seen.values() for i, _ in s['pigs'] + s['stands']]
    assert len(set(ids)) == len(ids), f'ids shared across the levels: {seen}'
    return seen


def level_rejoin(host, guest, out):
    """The guest leaves in the Nether and joins again there (its player data keeps its level)."""
    before = pos_of(guest)
    port = host.raw('lan', 'status')['port']
    guest.raw('disconnect')
    time.sleep(2)
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    after = pos_of(guest)
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert all(abs(a - b) < 1 for a, b in zip(before, after)), f'left at {before}, back at {after}'
    return {'at': after}


def level_death(host, guest, out):
    """The guest dies in the Nether and respawns in the Overworld; the host stays where it is."""
    guest('command', command='kill @s')
    guest.wait(20)
    guest.raw('respawn')
    back = wait_dimension(guest, OVERWORLD, 400)
    guest.wait(20)
    assert back.get('dimension') == OVERWORLD, f'the guest respawned in {back.get("dimension")}'
    assert dims(host, guest) == (OVERWORLD, OVERWORLD), f'host, guest in {dims(host, guest)}'
    return {'respawned': back['player']['position']}


def level_respawn_points(host, guest, out):
    """Respawn points outside the Overworld (ServerPlayer.findRespawnAndUseSpawnBlock): a point
    `spawnpoint` sets in the Nether (forced), then a charged respawn anchor the guest uses there,
    each bring the guest back in the Nether; the anchor loses a charge."""
    home = pos_of(guest)
    x, z = (int(v // 1) for v in home[::2])
    into_level(guest, NETHER, x, 100, z)
    results = {}
    try:
        guest('command', command=f'spawnpoint @s {x} 100 {z}')
        results['spawnpoint'] = die_and_respawn(guest)
        guest('command', command=f'setblock {x + 2} 100 {z} minecraft:respawn_anchor[charges=2]')
        guest.wait(10)
        guest.raw('interact', x + 2, 100, z)
        guest.wait(10)
        results['anchor'] = die_and_respawn(guest)
        results['charges'] = guest.raw('block', x + 2, 100, z).get('state')
    finally:
        sx, sy, sz = (int(v // 1) for v in pos_of(host))
        guest('command', command=f'execute in {OVERWORLD} run spawnpoint @s {sx} {sy} {sz}')
        guest('command', command=f'execute in {NETHER} run setblock {x + 2} 100 {z} minecraft:air')
        guest('command', command=f'execute in {OVERWORLD} run tp @s {home[0]:.2f} {home[1]:.2f} {home[2]:.2f}')
        wait_dimension(guest, OVERWORLD, 400)
    point, anchor = results['spawnpoint'], results['anchor']
    assert point[0] == NETHER and abs(point[1][0] - x - 0.5) < 0.6 and abs(point[1][2] - z - 0.5) < 0.6, \
        f'the spawnpoint brought the guest to {point}'
    assert anchor[0] == NETHER and abs(anchor[1][0] - x - 2.5) < 2 and abs(anchor[1][2] - z - 0.5) < 2, \
        f'the anchor brought the guest to {anchor}'
    assert 'charges=1' in (results['charges'] or ''), f'the anchor after the respawn: {results["charges"]}'
    return results


def die_and_respawn(guest):
    """Kill the guest and respawn it: the dimension and position it comes back at, once settled."""
    guest('command', command='kill @s')
    guest.wait(20)
    guest.raw('respawn')
    guest.wait(80)
    state = guest('state')
    return state.get('dimension'), state['player']['position']


def level_host_nether(host, guest, out):
    """The host goes through a Nether portal while the guest stays in the Overworld."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    nether_portal(host, hx, hy, hz - 3)
    try:
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz - 2.5}')
        there = wait_dimension(host, NETHER, 600)
        host.wait(20)
        result = dims(host, guest)
        assert there.get('dimension') == NETHER, f'the host is in {there.get("dimension")}'
        assert result == (NETHER, OVERWORLD), f'host, guest in {result}'
        return {'host': there['player']['position']}
    finally:
        host('command', command=f'execute in {OVERWORLD} run fill {hx - 1} {hy} {hz - 3} {hx + 2} {hy + 3} {hz - 3} '
                                'minecraft:air')
        host('command', command=f'execute in {OVERWORLD} run tp @s {hx + 0.5} {hy} {hz + 0.5}')
        wait_dimension(host, OVERWORLD, 400)


def level_guest_end(host, guest, out):
    """The guest steps into an End portal alone and lands on the obsidian platform, an End gateway
    takes it across the End, and an exit portal takes it back to the Overworld."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {x + 2} {y} {z} minecraft:end_portal')
    try:
        guest('command', command=f'tp @s {x + 2.5} {y} {z + 0.5}')
        wait_dimension(guest, END, 600)
        # The arrival waits for the platform's chunks; its portal cooldown (10 ticks) runs out after,
        # and a player who stands in a portal while it runs stays (Entity.setAsInsidePortal).
        guest.wait(60)
        landed = pos_of(guest)
        assert dims(host, guest) == (OVERWORLD, END), f'host, guest in {dims(host, guest)}'
        assert [int(v // 1) for v in landed] == [100, 49, 0], f'the guest landed at {landed}'
    finally:
        guest('command', command=f'execute in {OVERWORLD} run setblock {x + 2} {y} {z} minecraft:air')
    # A gateway far out with an exact exit at the End spawn, as end_gateway_return's
    # (TheEndGatewayBlockEntity.getPortalPosition), takes the guest back there within the End.
    guest('command', command='tp @s 700.5 80 0.5')
    guest.wait(60)
    level_floor(guest, 700, 80, 0)
    guest('command', command='tp @s 700.5 80 0.5')
    guest.wait(20)
    guest('command', command='setblock 702 80 0 minecraft:end_gateway{exit_portal:[I;100,50,0],exact_teleport:1b}')
    guest('command', command='tp @s 702.5 80 0.5')
    gateway = None
    for _ in range(20):
        guest.wait(10)
        gateway = pos_of(guest)
        if abs(gateway[0] - 702.5) > 50:
            break
    assert dims(host, guest) == (OVERWORLD, END), f'host, guest in {dims(host, guest)}'
    assert abs(gateway[0] - 100.5) < 1 and abs(gateway[2] - 0.5) < 1 and 49 <= gateway[1] <= 50, \
        f'the gateway left the guest at {gateway}'
    guest.wait(40)
    ex, ey, ez = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {ex + 2} {ey} {ez} minecraft:end_portal')
    guest('command', command=f'tp @s {ex + 2.5} {ey} {ez + 0.5}')
    home = wait_dimension(guest, OVERWORLD, 600)
    guest.wait(20)
    assert dims(host, guest) == (OVERWORLD, OVERWORLD), f'host, guest in {dims(host, guest)}'
    return {'landed': landed, 'gateway': gateway, 'home': home['player']['position']}


def level_reload(host, guest, out):
    """The host saves and quits with the guest in the Nether, opens the world again and opens it to the
    LAN; the guest joins again where it was, in the Nether."""
    x, z = (int(v // 1) for v in pos_of(guest)[::2])
    into_level(guest, NETHER, x, 100, z)
    before = pos_of(guest)
    host.raw('disconnect')
    # The world saves on its server thread before the title screen shows.
    for attempt in range(60):
        time.sleep(1)
        try:
            host.raw('open', 'Lan')
            break
        except Failed:
            if attempt == 59:
                raise
    host('wait_ready', seconds=400)
    port = host.raw('lan', 'open', 0, 'commands')['port']
    back_to_game(guest)
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    after = pos_of(guest)
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert all(abs(a - b) < 1 for a, b in zip(before, after)), f'left at {before}, back at {after}'
    return {'at': after}


LEVEL_STEPS = [('execute', level_execute), ('ticking', level_ticking), ('rejoin', level_rejoin),
               ('death', level_death), ('respawn_points', level_respawn_points),
               ('host_nether', level_host_nether),
               ('guest_end', level_guest_end), ('reload', level_reload)]


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


def mob_looks(game, centre):
    """What `check_looks` compares of each mob around `centre`, by kind and column."""
    found = {}
    for m in game.raw('mobs', 32).get('near', []):
        if abs(m['pos'][0] - centre[0]) > 12 or abs(m['pos'][2] - centre[2]) > 14:
            continue
        key = f"{m['kind'].removeprefix('minecraft:')}@{round(m['pos'][0] - centre[0])}"
        found[key] = {'baby': m['baby'], 'color': m['color'], 'sheared': m['sheared'], 'size': m['size'],
                      'tame': m['tame'], 'sitting': m['sitting'], 'burning': m['burning'],
                      'health': round(m['health'])}
    return found


def check_looks(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    centre = (x + 0.5, y, z + 0.5)
    # Each mob in a column of its own (the key), standing on the stone floor.
    looks = {
        'cow@-5': ('{NoAI:1b,Fire:80s}', {'burning': True}),
        'cow@-3': ('{NoAI:1b,Age:-24000}', {'baby': True}),
        'cow@-1': ('{NoAI:1b}', {'baby': False}),
        'sheep@1': ('{NoAI:1b,Color:14b,Sheared:1b}', {'color': 14, 'sheared': True}),
        'slime@3': ('{NoAI:1b,Size:3}', {'size': 4}),
        'wolf@5': ('{NoAI:1b,Owner:[I;1,2,3,4],Sitting:1b,CollarColor:11b}',
                   {'tame': True, 'sitting': True, 'color': 11}),
    }
    sides = {'host': host, 'guest': guest}
    seen = {}
    try:
        for key, (tag, _) in looks.items():
            kind, dx = key.split('@')
            host('command', command=f'summon minecraft:{kind} {centre[0] + int(dx)} {y} {z + 4.5} {tag}')
        host('command', command=f'summon minecraft:wither {centre[0]} {y + 3} {z + 10.5} {{NoAI:1b,Invul:0}}')
        host.wait(30)
        seen['spawned'] = {name: mob_looks(g, centre) for name, g in sides.items()}
        # Changes after the spawn: the wither hurt, the sheep dyed and grown back, the calf grown up.
        host('command', command='damage @e[type=minecraft:wither,limit=1] 150 minecraft:generic')
        for kind, dx, tag in (('sheep', 1, '{Color:11b,Sheared:0b}'), ('cow', -3, '{Age:0}')):
            host('command', command=f'data merge entity @e[type=minecraft:{kind},x={centre[0] + dx},y={y},'
                                    f'z={z + 4.5},distance=..0.5,limit=1] {tag}')
        guest.raw('look-at', centre[0], y + 1, z + 4.5)
        host.raw('look-at', centre[0], y + 1, z + 4.5)
        # The fire (80 ticks) goes out in the meantime.
        host.wait(80)
        for name, g in sides.items():
            g.shot(out, 'looks')
        seen['later'] = {name: mob_looks(g, centre) for name, g in sides.items()}
    finally:
        # A killed slime splits: shrink it first so no children are left hopping into the next check.
        host('command', command=f'data merge entity @e[type=minecraft:slime,x={centre[0] + 3},y={y},z={z + 4.5},'
                                f'distance=..2,limit=1] {{Size:0}}')
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 12},y={y - 2},z={z - 2},dx=24,dy=12,dz=16]')
    wrong = []
    for name in sides:
        spawned, later = seen['spawned'][name], seen['later'][name]
        for key, (_, want) in looks.items():
            got = spawned.get(key)
            if got is None or any(got[k] != v for k, v in want.items()):
                wrong.append(f'{name} {key}: {got}, wants {want}')
        if later.get('cow@-5', {}).get('burning') is not False:
            wrong.append(f"{name}: the cow's fire never went out: {later.get('cow@-5')}")
        if [later.get('sheep@1', {}).get(k) for k in ('color', 'sheared')] != [11, False]:
            wrong.append(f"{name}: the sheep dyed blue with its wool back: {later.get('sheep@1')}")
        if later.get('cow@-3', {}).get('baby') is not False:
            wrong.append(f"{name}: the calf grown up: {later.get('cow@-3')}")
        withers = [v['health'] for k, v in spawned.items() if k.startswith('wither')]
        hurt = [v['health'] for k, v in later.items() if k.startswith('wither')]
        if withers != [300] or hurt != [150]:
            wrong.append(f"{name}: the wither's health {withers} then {hurt}, wants [300] then [150]")
    assert not wrong, '\n'.join(wrong)
    return seen['later']['guest']


def check_leads(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        at = (x + 0.5, y, z + 2.5)

        def cow(game):
            found = [m for m in game.raw('mobs', 32).get('near', []) if m['kind'] == 'minecraft:cow'
                     and abs(m['pos'][0] - at[0]) < 1 and abs(m['pos'][2] - at[2]) < 1]
            assert found, f'{game.name} sees no cow at {at}'
            return found[0]
        player('command', command='gamemode survival')
        player('command', command=f'summon minecraft:cow {at[0]} {y} {at[2]} {{NoAI:1b}}')
        player('command', command='item replace entity @s weapon.mainhand with minecraft:lead')
        player.wait(30)
        try:
            player.raw('look-at', f'{at[0]:.2f}', f'{y + 0.7:.2f}', f'{at[2]:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(20)
            tied = {'own': cow(player)['leash'], 'other': cow(other)['leash']}
            player.shot(out, 'leads-' + label)
            other.shot(out, f'leads-{label}-seen')
            # An empty hand unties it (the lead was used up).
            player.raw('mouse', 'right', 'click')
            player.wait(20)
            untied = {'own': cow(player)['leash'], 'other': cow(other)['leash']}
        finally:
            player('command', command=f'kill @e[type=!minecraft:player,x={at[0]},y={y},z={at[2]},distance=..6]')
            player('command', command='clear @s')
        result[label] = {'tied': tied, 'untied': untied}
        # The other side draws it held by the player who tied it (another player to it).
        assert tied['own'] == 'Player' and tied['other'], f"the {label}'s cow on its lead: {tied}"
        assert untied == {'own': None, 'other': None}, f"the {label}'s cow untied: {untied}"
    return result


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


def open_station(guest, block, out, label):
    """Place `block` two blocks south of the guest and open it: the station's position."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'station', '--timeout', 10)
    guest.wait(10)
    guest.shot(out, label)
    return at


def station_slots(guest):
    """The open station's stacks: input slots 600 and up, the result at 650."""
    menu = guest.raw('menu')
    return {s['slot']: (s['stack']['item'], s['stack']['count']) for s in menu['slots'] if s['slot'] >= 600}


def close_station(guest, at):
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')


def check_loom(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    for item in ('white_banner', 'red_dye'):
        guest('command', command=f'give @s minecraft:{item}')
    at = open_station(guest, 'loom', out, 'loom')
    # LoomMenu.quickMoveStack: a banner to the banner slot, a dye to the dye slot.
    for item in ('white_banner', 'red_dye'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    loaded = station_slots(guest)
    # LoomScreen's first pattern button (clickMenuButton 0): the server makes the result.
    guest.raw('click-gui', 67, 20)
    guest.wait(10)
    made = station_slots(guest).get(650)
    guest.shot(out, 'loom-pattern')
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    # The server's copy of the banner the guest took has the red layer.
    since = guest.raw('events')['last']
    guest('command', command='data get entity @s Inventory[{id:"minecraft:white_banner"}].components')
    layer = guest.raw('wait-for', 'chat', 'color: "red"', '--timeout', 10, '--since', since).get('ok')
    counts = guest('inventory')['counts']
    seen = {'loaded': loaded, 'made': made, 'left': left, 'layer': layer, 'counts': counts}
    want = {'loaded': {600: ('minecraft:white_banner', 1), 601: ('minecraft:red_dye', 1)},
            'made': ('minecraft:white_banner', 1), 'left': {}, 'layer': True,
            'counts': {'minecraft:white_banner': 1}}
    assert seen == want, f'the loom on the guest: {seen}'
    return seen


def own_reply(event):
    """A chat event that answers the guest's own command. On the server thread the guest also gets
    its own command's `[Guest: ...]` operator broadcast, a moment later, which an answer to the next
    command must not be taken for."""
    return event.get('event') == 'chat' and not event.get('text', '').startswith(f'[{GUEST_NAME}: ')


def server_count(guest, item):
    """How many of `item` the server says the guest has (`clear` with a count of 0 only counts)."""
    since = guest.raw('events')['last']
    guest('command', command=f'clear @s minecraft:{item} 0')
    for n in range(0, 65):
        reply = guest.raw('events', '--since', since)
        lines = [e.get('text', '') for e in reply['events'] if own_reply(e)]
        for line in lines:
            if 'matching item' in line or 'No items were found' in line:
                return 0 if 'No items' in line else int(line.split('Found ')[1].split(' ')[0])
        guest.wait(2)
    raise Failed(f'no answer to clear for {item}')


def check_stonecutter(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:stone 3')
    at = open_station(guest, 'stonecutter', out, 'stonecutter')
    # StonecutterMenu.quickMoveStack: stone has recipes, so it goes to the input.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:stone'), 'shift')
    guest.wait(10)
    recipes = guest.raw('menu')['info']['recipes']
    index = recipes.index('minecraft:stone_slab_from_stone_stonecutting')
    # StonecutterScreen's recipe grid: four columns of 16x18 cells from (52, 14).
    guest.raw('click-gui', 52 + index % 4 * 16 + 8, 14 + index // 4 * 18 + 9)
    guest.wait(10)
    picked = (guest.raw('menu')['info']['selected'], station_slots(guest).get(650))
    guest.shot(out, 'stonecutter-recipe')
    # A shift-click on the result cuts while there is stone (doClick repeats quickMoveStack).
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    seen = {'picked': picked, 'left': left,
            'server': {item: server_count(guest, item) for item in ('stone_slab', 'stone')}}
    want = {'picked': (index, ('minecraft:stone_slab', 2)), 'left': {},
            'server': {'stone_slab': 6, 'stone': 0}}
    assert seen == want, f'the stonecutter on the guest: {seen}'
    return seen


def server_reply(guest, command, *needles):
    """The first chat line holding every needle after the guest runs `command`."""
    since = guest.raw('events')['last']
    guest('command', command=command)
    for _ in range(0, 65):
        reply = guest.raw('events', '--since', since)
        for e in reply['events']:
            line = e.get('text', '')
            if own_reply(e) and all(n in line for n in needles):
                return line
        guest.wait(2)
    raise Failed(f'no answer to {command}')


def check_grindstone(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 0 levels')
    guest('command', command='xp set @s 0 points')
    guest('command', command='give @s minecraft:iron_sword[minecraft:enchantments={"minecraft:sharpness":3,'
                             '"minecraft:vanishing_curse":1},minecraft:damage=100]')
    at = open_station(guest, 'grindstone', out, 'grindstone')
    # GrindstoneMenu.quickMoveStack: with both inputs empty, into the first.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:iron_sword'), 'shift')
    guest.wait(10)
    loaded = station_slots(guest)
    guest.shot(out, 'grindstone-result')
    guest.raw('click-slot', 650, 'shift')
    # The orbs (sharpness III: 12 to 23 points) fly to the guest.
    guest.wait(60)
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}].components',
                         'minecraft:damage')
    # 16 points make exactly two levels and leave 0 points: count the levels too.
    xp = (server_reply(guest, 'xp query @s points', 'experience points')
          + server_reply(guest, 'xp query @s levels', 'experience levels'))
    seen = {'loaded': loaded, 'left': left, 'curse kept': 'vanishing_curse' in sword,
            'sharpness gone': 'sharpness' not in sword,
            'damage': re.search(r'minecraft:damage"?: 100\b', sword) is not None,
            'xp': any(int(n) > 0 for n in re.findall(r'has (\d+) experience', xp))}
    want = {'loaded': {600: ('minecraft:iron_sword', 1), 650: ('minecraft:iron_sword', 1)}, 'left': {},
            'curse kept': True, 'sharpness gone': True, 'damage': True, 'xp': True}
    assert seen == want, f'the grindstone on the guest: {seen} ({sword!r}, {xp!r})'
    return seen


def check_anvil(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 5 levels')
    guest('command', command='give @s minecraft:iron_sword[minecraft:damage=200]')
    guest('command', command='give @s minecraft:iron_ingot 3')
    at = open_station(guest, 'anvil', out, 'anvil')
    # ItemCombinerMenu.quickMoveStack: into the first input that takes it.
    for item in ('iron_sword', 'iron_ingot'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    # The box shows "Iron Sword": erase it and type the new name.
    for _ in range(len('Iron Sword')):
        guest.raw('type-back')
    guest.raw('type', 'Blade')
    guest.wait(10)
    menu = guest.raw('menu')
    offered = (menu['info']['anvil_cost'], station_slots(guest).get(650))
    guest.shot(out, 'anvil-rename')
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}].components',
                         'minecraft:damage')
    levels = server_reply(guest, 'xp query @s levels', 'experience levels')
    seen = {'offered': offered, 'left': left, 'name': 'Blade' in sword,
            'damage': re.search(r'minecraft:damage"?: 14\b', sword) is not None,
            'levels': int(re.search(r'has (\d+) experience', levels).group(1)),
            'ingots': server_count(guest, 'iron_ingot')}
    # Three ingots repair 62 each (a quarter of 250), plus the name: four levels.
    want = {'offered': (4, ('minecraft:iron_sword', 1)), 'left': {}, 'name': True, 'damage': True,
            'levels': 1, 'ingots': 0}
    assert seen == want, f'the anvil on the guest: {seen} ({sword!r})'
    return seen


def check_smithing(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    for item in ('netherite_upgrade_smithing_template', 'diamond_sword', 'netherite_ingot'):
        guest('command', command=f'give @s minecraft:{item}')
    at = open_station(guest, 'smithing_table', out, 'smithing')
    # SmithingMenu.canMoveIntoInputSlots: each to the slot whose recipe test takes it.
    for item in ('netherite_upgrade_smithing_template', 'diamond_sword', 'netherite_ingot'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(5)
    loaded = station_slots(guest)
    guest.shot(out, 'smithing-result')
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    seen = {'loaded': loaded, 'left': left,
            'server': {item: server_count(guest, item) for item in ('netherite_sword', 'diamond_sword', 'netherite_ingot')}}
    want = {'loaded': {600: ('minecraft:netherite_upgrade_smithing_template', 1), 601: ('minecraft:diamond_sword', 1),
                       602: ('minecraft:netherite_ingot', 1), 650: ('minecraft:netherite_sword', 1)},
            'left': {}, 'server': {'netherite_sword': 1, 'diamond_sword': 0, 'netherite_ingot': 0}}
    assert seen == want, f'the smithing table on the guest: {seen}'
    return seen


def check_enchanting(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 30 levels')
    guest('command', command='give @s minecraft:iron_sword')
    guest('command', command='give @s minecraft:lapis_lazuli 3')
    # EnchantmentMenu awards the enchant_item stat; a stat objective shows the guest's count.
    guest('command', command='scoreboard objectives add lanench minecraft.custom:minecraft.enchant_item')
    at = open_station(guest, 'enchanting_table', out, 'enchanting')
    # EnchantmentMenu.quickMoveStack: the sword to the item slot, the lapis to its own.
    for item in ('iron_sword', 'lapis_lazuli'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(10)
    costs = guest.raw('menu')['info']['costs']
    guest.shot(out, 'enchanting-offers')
    # EnchantmentScreen's first offer button (clickMenuButton 0).
    guest.raw('click-gui', 110, 23)
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}]', 'iron_sword')
    levels = server_reply(guest, 'xp query @s levels', 'experience levels')
    stat = server_reply(guest, 'scoreboard players get @s lanench', 'lanench')
    guest('command', command='scoreboard objectives remove lanench')
    seen = {'offered': costs[0] > 0, 'enchanted': 'minecraft:enchantments' in sword,
            'left': left.get(601), 'levels': int(re.search(r'has (\d+) experience', levels).group(1)),
            'lapis': server_count(guest, 'lapis_lazuli'), 'stat': ' has 1 ' in stat}
    want = {'offered': True, 'enchanted': True, 'left': ('minecraft:lapis_lazuli', 2), 'levels': 29, 'lapis': 2,
            'stat': True}
    assert seen == want, f'the enchanting table on the guest: {seen} ({costs}, {sword!r})'
    return seen


def check_beacon(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='effect clear @s')
    guest('command', command='give @s minecraft:iron_ingot')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    # A one-level pyramid under the beacon; its levels come with the first base check (80 ticks).
    guest('command', command=f'fill {x - 1} {y - 1} {z + 1} {x + 1} {y - 1} {z + 3} minecraft:iron_block')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:beacon')
    guest.wait(100)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'station', '--timeout', 10)
    guest.wait(10)
    # BeaconMenu.quickMoveStack: the ingot to the payment slot.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:iron_ingot'), 'shift')
    guest.wait(10)
    paid = station_slots(guest).get(600)
    # BeaconScreen (54 px wider, 27 to the left): Haste, the first level's second power, then Done,
    # which sends ServerboundSetBeaconPacket and closes the menu.
    guest.raw('click-gui', 77 - 27 + 11, 22 + 11)
    guest.wait(5)
    guest.shot(out, 'beacon')
    guest.raw('click-gui', 164 - 27 + 11, 107 + 11)
    guest.wait(10)
    screen = guest('state').get('screen')
    effects = []
    # BeaconBlockEntity.applyEffects runs every 80 ticks.
    for _ in range(12):
        guest.wait(20)
        effects = effect_ids(guest)
        if 'haste' in effects:
            break
    seen = {'paid': paid, 'haste': 'haste' in effects, 'ingots': server_count(guest, 'iron_ingot'),
            'screen': screen}
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    want = {'paid': ('minecraft:iron_ingot', 1), 'haste': True, 'ingots': 0, 'screen': 'playing'}
    assert seen == want, f'the beacon on the guest: {seen} ({effects})'
    return seen


def check_crafter(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_planks 4')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:crafter')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    # CrafterScreen.slotClicked: an empty-handed click disables the first slot
    # (ServerboundContainerSlotStateChangedPacket); the planks then skip it.
    guest.raw('click-slot', 300)
    guest.wait(5)
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:oak_planks'), 'shift')
    guest.wait(10)
    guest.shot(out, 'crafter')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # Opening it again shows the server's crafter.
    items = open_container(guest, at, out, 'crafter-after')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'items': items[0] if isinstance(items, tuple) else items, 'planks': server_count(guest, 'oak_planks')}
    want = {'items': {1: 'minecraft:oak_planks'}, 'planks': 0}
    assert seen == want, f'the crafter on the guest: {seen}'
    return seen


def check_lectern(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    book = ('{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":'
            '{title:"Lan",author:"Host",pages:["one","two","three"]}}}')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:lectern[has_book=true]{{Book:{book}}}')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'book', '--timeout', 10)
    guest.wait(10)
    opened = guest.raw('book')
    guest.shot(out, 'lectern')
    # LecternScreen's page arrows: ContainerButtonClick 2 (forward) and 1 (back); the server's
    # LecternMenu.setData clamps to the book's last page and sends the page back (data slot 0).
    for button in ('next', 'next', 'next', 'back'):
        guest.raw('book', button)
        guest.wait(5)
    guest.wait(10)
    turned = guest.raw('book')
    page = server_reply(guest, f'data get block {at[0]} {at[1]} {at[2]} Page', 'block data')
    guest.shot(out, 'lectern-page')
    # Take Book: ContainerButtonClick 3; the server empties the lectern and closes the menu.
    guest.raw('book', 'take')
    guest.wait(20)
    screen = guest('state').get('screen')
    held = server_count(guest, 'written_book')
    has_book = server_reply(guest, f'execute if block {at[0]} {at[1]} {at[2]} minecraft:lectern[has_book=false]',
                            'Test passed')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'opened': (opened.get('page'), opened.get('pages')), 'turned': turned.get('page'),
            'server page': int(re.search(r'(-?\d+)\s*$', page).group(1)), 'screen': screen, 'held': held,
            'emptied': 'Test passed' in has_book}
    want = {'opened': (0, 3), 'turned': 1, 'server page': 1, 'screen': 'playing', 'held': 1, 'emptied': True}
    assert seen == want, f'the lectern on the guest: {seen} ({page!r})'
    return seen


def check_cartography(host, guest, out):
    # The host makes a map (EmptyMapItem.use), the guest gets a copy of it.
    host('command', command='clear @s')
    host('command', command='item replace entity @s weapon.mainhand with minecraft:map')
    host.wait(5)
    host.raw('use-item')
    host.wait(20)
    made = server_reply(host, 'data get entity @s Inventory[{id:"minecraft:filled_map"}].components', 'map_id')
    map_id = int(re.search(r'map_id"?: (\d+)', made).group(1))
    host('command', command='clear @s')
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command=f'give @s minecraft:filled_map[minecraft:map_id={map_id}]')
    guest('command', command='give @s minecraft:paper')
    at = open_station(guest, 'cartography_table', out, 'cartography')
    # CartographyTableMenu.quickMoveStack: a map with an id to the map slot, paper to the other.
    for item in ('filled_map', 'paper'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(10)
    loaded = station_slots(guest)
    guest.shot(out, 'cartography-result')
    # The result's onTake uses the map and the paper; MapItem.onCraftedPostProcess gives the zoomed
    # out map a new id on the server.
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    taken = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:filled_map"}].components', 'map_id')
    new_id = int(re.search(r'map_id"?: (\d+)', taken).group(1))
    seen = {'loaded': loaded, 'left': left, 'new map': new_id != map_id,
            'server': {item: server_count(guest, item) for item in ('filled_map', 'paper')}}
    want = {'loaded': {600: ('minecraft:filled_map', 1), 601: ('minecraft:paper', 1),
                       650: ('minecraft:filled_map', 1)},
            'left': {}, 'new map': True, 'server': {'filled_map': 1, 'paper': 0}}
    assert seen == want, f'the cartography table on the guest: {seen} ({made!r}, {taken!r})'
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


def check_thrown(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        for item, kind in (('splash_potion', 'minecraft:splash_potion'), ('ender_eye', 'minecraft:eye_of_ender')):
            player('command', command=f'item replace entity @s weapon.mainhand with minecraft:{item}')
            player.wait(10)
            # Up at 45 degrees, into open sky.
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 11.6:.2f}', f'{z + 10.5:.2f}')
            player.raw('use-item')
            seen = {'own': 0, 'other': 0}
            for _ in range(10):
                player.wait(2)
                for side, game in (('own', player), ('other', other)):
                    if any(p['kind'] == kind for p in game('status')['projectiles']):
                        seen[side] += 1
                if all(seen.values()):
                    break
            player.shot(out, f'thrown-{label}-{item}')
            other.shot(out, f'thrown-{label}-{item}-seen')
            player.wait(100)  # it lands or drops
            result[f'{label} {item}'] = seen
    for name, seen in result.items():
        assert all(seen.values()), f'{name}: seen in flight (samples) {result}'
    return result


def made(game, kind):
    """How many `kind` particles `game` has made so far."""
    return game('status').get('particles', {}).get(kind, 0)


def sparked(player, other, kinds, act, ticks=20):
    """The `kinds` particles each side makes while `act` runs and for up to `ticks` after."""
    before = {(side, k): made(g, k) for side, g in (('own', player), ('other', other)) for k in kinds}
    act()
    seen = {}
    for _ in range(ticks // 2):
        player.wait(2)
        seen = {f'{side} {k}': made(g, k) - before[(side, k)]
                for side, g in (('own', player), ('other', other)) for k in kinds}
        if all(seen.values()):
            break
    return seen


def check_sparks(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        at = (x, y, z + 2)
        player('command', command='gamemode survival')
        try:
            player('command', command=f'setblock {at[0]} {at[1] - 1} {at[2]} minecraft:farmland')
            player('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:wheat')
            player('command', command='item replace entity @s weapon.mainhand with minecraft:bone_meal 8')
            player.wait(10)

            def click(height):
                player.raw('look-at', f'{at[0] + 0.5:.2f}', f'{at[1] + height:.2f}', f'{at[2] + 0.5:.2f}')
                player.raw('mouse', 'right', 'click')
            result[f'{label} bone meal'] = sparked(player, other, ['happy_villager'], lambda: click(0.1))
            player('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:copper_block')
            player('command', command='item replace entity @s weapon.mainhand with minecraft:honeycomb')
            player.wait(10)
            result[f'{label} wax'] = sparked(player, other, ['wax_on'], lambda: click(0.5))
            player('command', command='item replace entity @s weapon.mainhand with minecraft:iron_axe')
            player.wait(10)
            result[f'{label} wax off'] = sparked(player, other, ['wax_off'], lambda: click(0.5))
            block = player.raw('block', *at).get('block', '')
            result[f'{label} copper'] = block
        finally:
            player('command', command=f'fill {at[0]} {at[1] - 1} {at[2]} {at[0]} {at[1]} {at[2]} minecraft:air')
            player('command', command=f'setblock {at[0]} {at[1] - 1} {at[2]} minecraft:stone')
            player('command', command='clear @s')
    # A spawner beside the host, at night: its burst when the zombie comes.
    x, y, z = spots['host']
    host('command', command='time set midnight')
    try:
        host('command', command=f'setblock {x} {y} {z + 3} minecraft:spawner{{Delay:20s,SpawnCount:1s,'
                                'SpawnData:{entity:{id:"minecraft:zombie",NoAI:1b}}}')
        result['spawner'] = sparked(host, guest, ['flame', 'smoke'], lambda: None, ticks=200)
    finally:
        host('command', command=f'setblock {x} {y} {z + 3} minecraft:air')
        host('command', command=f'kill @e[type=minecraft:zombie,x={x},y={y},z={z},distance=..12]')
        host('command', command='time set day')
    wrong = {k: v for k, v in result.items() if isinstance(v, dict) and not all(n > 0 for n in v.values())}
    assert not wrong, f'no particles: {wrong}\n{result}'
    return result


def hearts(game):
    """The player's maximum and current health as its own client holds them."""
    p = game('status')['player']
    return p.get('max_health'), round(p['health'], 1)


def check_hud(host, guest, out):
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        player('command', command='gamemode survival')
        try:
            for maximum in (40, 10):
                player('command', command=f'attribute @s minecraft:max_health base set {maximum}')
                player.wait(10)
                player('command', command='effect give @s minecraft:instant_health 1 10')
                seen = None
                for _ in range(15):
                    player.wait(2)
                    seen = hearts(player)
                    if seen == (maximum, maximum):
                        break
                result[f'{label} {maximum}'] = seen
                player.shot(out, f'hearts-{label}-{maximum}')
        finally:
            player('command', command='attribute @s minecraft:max_health base reset')
            player('command', command='effect give @s minecraft:instant_health 1 10')
    # BossBarCommands' new bar starts visible (ServerBossEvent's constructor).
    host('command', command='bossbar add lan_e2e:bar "LAN bar"')
    try:
        host('command', command='bossbar get lan_e2e:bar visible')
        host.wait(5)
        result['bar'] = [line for line in host.raw('messages', 5)['chat'] if 'lan_e2e:bar' in line or 'LAN bar' in line]
    finally:
        host('command', command='bossbar remove lan_e2e:bar')
    wrong = {k: v for k, v in result.items() if k != 'bar' and v != (int(k.split()[1]),) * 2}
    assert not wrong, f'hearts: {wrong}\n{result}'
    assert any('currently visible' in line for line in result['bar']), f'bar: {result["bar"]}'
    return result


def sidebars(host, guest, want, ticks=40):
    """Each side's sidebar (objective and scores) once both show `want` (host, guest), or after `ticks`."""
    seen = None
    for _ in range(ticks // 2):
        host.wait(2)
        seen = tuple(g('status').get('sidebar') for g in (host, guest))
        got = tuple(s and (s['objective'], s['scores']) for s in seen)
        if got == want:
            break
    return seen


def check_sidebar(host, guest, out):
    result = {}
    run = lambda *cmds: [host('command', command=c) for c in cmds]
    try:
        run('scoreboard objectives add audit dummy {"text":"Audit board","color":"aqua"}',
            'scoreboard players set Alpha audit 7', 'scoreboard players set Beta audit 3',
            'scoreboard objectives setdisplay sidebar audit')
        both = ('audit', {'Alpha': 7, 'Beta': 3})
        result['shown'] = sidebars(host, guest, (both, both))
        host.shot(out, 'sidebar')
        guest.shot(out, 'sidebar')
        run('scoreboard players set Alpha audit 12', 'scoreboard players reset Beta audit')
        both = ('audit', {'Alpha': 12})
        result['changed'] = sidebars(host, guest, (both, both))
        # The guest's team's sidebar replaces the sidebar for the guest only.
        run('team add red', 'team modify red color red', f'team join red {GUEST_NAME}',
            'scoreboard objectives add reds dummy', f'scoreboard players set {GUEST_NAME} reds 5',
            'scoreboard objectives setdisplay sidebar.team.red reds')
        result['team'] = sidebars(host, guest, (both, ('reds', {GUEST_NAME: 5})))
        run('scoreboard objectives remove reds', 'scoreboard objectives setdisplay sidebar')
        result['cleared'] = sidebars(host, guest, (None, None))
    finally:
        run('scoreboard objectives remove audit', 'scoreboard objectives remove reds', 'team remove red')
    want = {'shown': (('audit', {'Alpha': 7, 'Beta': 3}),) * 2, 'changed': (('audit', {'Alpha': 12}),) * 2,
            'team': (('audit', {'Alpha': 12}), ('reds', {GUEST_NAME: 5})), 'cleared': (None, None)}
    got = {k: tuple(s and (s['objective'], s['scores']) for s in v) for k, v in result.items()}
    wrong = {k: v for k, v in got.items() if v != want[k]}
    assert not wrong, f'sidebars: {wrong}\n{result}'
    return result


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


def check_drops(host, guest, out):
    """A dropped item lying where a block is then placed comes out of it (#1252): the server moves
    it towards the closest open side (ItemEntity.tick's noPhysics, Entity.moveTowardsClosestSpace),
    and both sides see it outside the block. The host places one, then the guest."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -3), (guest, 3)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command=f'fill {x - 1} {y} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        game('command', command=f'summon minecraft:item {x + 0.5} {y} {z + 0.5} '
                                '{Item:{id:"minecraft:diamond",count:1},PickupDelay:32767}')
        game.wait(20)
        game('command', command=f'setblock {x} {y} {z} minecraft:stone')
        game.wait(60)
        for viewer in (host, guest):
            near = [e['pos'] for e in viewer.raw('entities', 24, 'minecraft:item')['entities']
                    if abs(e['pos'][0] - x - 0.5) < 2 and abs(e['pos'][2] - z - 0.5) < 2]
            seen[f'{game.name} places, {viewer.name} sees'] = ([x, y, z], near)
        game.shot(out, 'drops')
        host('command', command='kill @e[type=minecraft:item]')
        host('command', command=f'setblock {x} {y} {z} minecraft:air')
    for name, (block, near) in seen.items():
        inside = [p for p in near if all(block[i] <= p[i] < block[i] + 1 for i in range(3))]
        assert near and not inside, f'{name}: items {near} in block {block} (all: {seen})'
    return seen


def check_underwater(host, guest, out):
    """Digging dirt by hand underwater, standing on the bottom of a pool: five times slower than in
    the air (SUBMERGED_MINING_SPEED), on the player's own client as on the server. Halfway through
    the client is about halfway (it doesn't break the block early, which the server would put
    back), and the block breaks (#1254). The dirt is at eye height, the player's head under water.
    The host, then the guest."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='gamemode survival')
        game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
        game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y + 3} {z + 1} minecraft:stone')
        game('command', command=f'fill {x} {y} {z} {x} {y + 2} {z} minecraft:water')
        game('command', command=f'setblock {x + 1} {y + 1} {z} minecraft:dirt')
        game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        game.wait(40)
        game.raw('look-at', f'{x + 1.5:.2f}', f'{y + 1.5:.2f}', f'{z + 0.5:.2f}')
        game.wait(5)
        game.raw('mouse', 'left', 'down')
        # Dirt by hand: 15 ticks in the air, 75 under water.
        game.wait(35)
        halfway = {'mining': game('state').get('mining'), 'client': game.raw('block', x + 1, y + 1, z).get('block'),
                   'server': host.raw('block', x + 1, y + 1, z).get('block'), 'y': y + 1}
        game.wait(70)
        game.raw('mouse', 'left', 'up')
        game.wait(10)
        done = host.raw('block', x + 1, y + 1, z).get('block')
        game.shot(out, 'underwater')
        seen[game.name] = {'halfway': halfway, 'done': done}
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y + 3} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y - 1} {z + 1} minecraft:stone')
    for name, s in seen.items():
        h = s['halfway']
        assert h['client'] == 'minecraft:dirt' == h['server'], f'{name} broke it early: {seen}'
        assert h['mining'] and h['mining']['pos'][1] == h['y'] and 0.3 < h['mining']['progress'] < 0.7, f'{name} digs at the wrong speed: {seen}'
        # Broken: air, or the pool's water flowing in.
        assert s['done'] in ('minecraft:air', 'minecraft:water'), f'{name} never broke it: {seen}'
    return seen





def own(game):
    """The player's own state, as its client has it."""
    return game('state')['player']




def check_contacts(host, guest, out):
    """Blocks that hurt what touches them, on the server: standing on magma (but not sneaking on it)
    and on a lit campfire, walking against a cactus and through grown sweet berry bushes, for the
    host and the guest (#1305, #1311, #1312, #1313); a young bush pricks no one."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='gamerule natural_health_regeneration false')
    seen = {}
    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, y, z = hx + 2, hy, hz + dz
            game('command', command='gamemode survival')
            game('command', command='effect clear @s')
            for case, blocks, walk in (
                    ('magma', [(0, -1, 'minecraft:magma_block')], False),
                    ('sneaking', [(0, -1, 'minecraft:magma_block')], False),
                    ('campfire', [(0, 0, 'minecraft:campfire[lit=true]')], False),
                    ('cactus', [(2, -1, 'minecraft:sand'), (2, 0, 'minecraft:cactus')], True),
                    ('berries', [(k, -1, 'minecraft:grass_block') for k in range(1, 4)]
                     + [(k, 0, 'minecraft:sweet_berry_bush[age=3]') for k in range(1, 4)], True),
                    ('young', [(k, -1, 'minecraft:grass_block') for k in range(1, 4)]
                     + [(k, 0, 'minecraft:sweet_berry_bush[age=0]') for k in range(1, 4)], True)):
                game('command', command='effect give @s minecraft:instant_health 1 5 true')
                game('command', command=f'fill {x - 1} {y - 2} {z - 1} {x + 4} {y - 1} {z + 1} minecraft:stone')
                game('command', command=f'fill {x - 1} {y} {z - 1} {x + 4} {y + 2} {z + 1} minecraft:air')
                # A wall ends the walk before the player leaves the floor.
                game('command', command=f'fill {x + 5} {y - 1} {z - 1} {x + 5} {y + 2} {z + 1} minecraft:stone')
                for dx, dy, block in blocks:
                    game('command', command=f'setblock {x + dx} {y + dy} {z} {block}')
                stand = 0.5 if case == 'campfire' else 0.0
                # Sneaking from before the player lands on the magma.
                held = ['shift'] if case == 'sneaking' else []
                for k in held:
                    game.raw('key', k, 'down')
                game.wait(5)
                game('command', command=f'tp @s {x + 0.5} {y + stand} {z + 0.5}')
                game.raw('look-at', f'{x + 10:.2f}', f'{y + 1.6:.2f}', f'{z + 0.5:.2f}')
                game.wait(30)
                before = own(game)['health']
                keys = ['w'] if walk else []
                for k in keys:
                    game.raw('key', k, 'down')
                game.wait(60)
                # Read before letting go: standing up on the magma would hurt.
                after = own(game)['health']
                for k in keys + held:
                    game.raw('key', k, 'up')
                game.wait(10)
                seen[f'{game.name}-{case}'] = (before, after)
                game.shot(out, f'contacts-{case}')
            host('command', command=f'fill {x - 1} {y} {z - 1} {x + 5} {y + 2} {z + 1} minecraft:air')
    finally:
        host('command', command='gamerule natural_health_regeneration true')
    for case, (before, after) in seen.items():
        safe = case.endswith(('sneaking', 'young'))
        assert (after == before) if safe else (after < before), f'{case}: {before} -> {after} (all: {seen})'
    return seen


def server_player(host, game):
    """The player's entry on the server: the guest's in the host's `lan status`, on the server
    thread the host's too; the host's own world otherwise."""
    if game is host and not SERVER_THREAD:
        return own(host)
    name = GUEST_NAME if game is not host else host.player
    return next(g for g in host.raw('lan', 'status')['guests'] if g['name'] == name)


def check_shield(host, guest, out):
    """An axe disables a raised shield for 5 s (100 ticks), counted down once a tick on the
    server: a vindicator's axe hits each player's shield, then the cooldown left falls by about 40
    over 40 ticks (#1316)."""
    host('command', command='gamerule natural_health_regeneration false')
    rates = {}
    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, y, z = (int(v // 1) for v in pos_of(host))
            z += dz
            game('command', command='gamemode survival')
            game('command', command='effect give @s minecraft:resistance 60 4 true')
            game('command', command='effect give @s minecraft:instant_health 1 5 true')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
            game('command', command='item replace entity @s weapon.offhand with minecraft:shield')
            game('command', command=f'fill {x - 1} {y} {z - 1} {x + 4} {y + 2} {z + 1} minecraft:air')
            game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 4} {y - 1} {z + 1} minecraft:stone')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            game.wait(10)
            game.raw('look-at', f'{x + 2.5:.2f}', f'{y + 1.5:.2f}', f'{z + 0.5:.2f}')
            game.raw('mouse', 'right', 'down')
            game.wait(10)
            host('command', command=f'summon minecraft:vindicator {x + 2.5} {y} {z + 0.5} '
                 '{PersistenceRequired:1b,Rotation:[90f,0f]}')
            host('command', command='item replace entity @e[type=minecraft:vindicator] weapon.mainhand '
                 'with minecraft:iron_axe')
            first = None
            for _ in range(60):
                game.wait(5)
                first = server_player(host, game).get('shield_cooldown', 0)
                if first:
                    break
            host('command', command='kill @e[type=minecraft:vindicator]')
            game.raw('mouse', 'right', 'up')
            assert first, f'{game.name}: the vindicator never disabled the shield'
            start = server_player(host, game)['shield_cooldown']
            game.wait(40)
            end = server_player(host, game)['shield_cooldown']
            rates[game.name] = {'first': first, 'start': start, 'end': end}
            game.shot(out, 'shield')
            game('command', command='item replace entity @s weapon.offhand with minecraft:air')
            game('command', command='effect clear @s')
    finally:
        host('command', command='kill @e[type=minecraft:vindicator]')
        host('command', command='gamerule natural_health_regeneration true')
    for name, r in rates.items():
        assert r['first'] <= 100, f'{name}: {rates}'
        # Once a tick: 40 down over 40 ticks, give or take the ticks the two queries straddle.
        assert 30 <= r['start'] - r['end'] <= 50, f'{name}: {rates}'
    return rates


def respawn_when_ready(game):
    """Press Respawn once the death screen lets it (DeathScreen's 20-tick delay, counted from when
    the screen opens)."""
    for _ in range(20):
        try:
            return game.raw('respawn')
        except Failed as e:
            if 'not active yet' not in str(e):
                raise
            game.wait(10)
    return game.raw('respawn')


def check_world_spawn(host, guest, out):
    """`setworldspawn` moves the world spawn: the host and the guest, with no bed of their own,
    respawn there once killed (#1310)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    sx, sz = hx + 24, hz
    host('command', command='gamerule respawn_radius 0')
    host('command', command=f'fill {sx - 2} {hy - 1} {sz - 2} {sx + 2} {hy - 1} {sz + 2} minecraft:stone')
    host('command', command=f'fill {sx - 2} {hy} {sz - 2} {sx + 2} {hy + 3} {sz + 2} minecraft:air')
    seen = {}
    try:
        host('command', command=f'setworldspawn {sx} {hy} {sz}')
        for game in (host, guest):
            game('command', command='gamemode survival')
            game('command', command='kill @s')
            game.wait(20)
            respawn_when_ready(game)
            game.wait(60)
            seen[game.name] = [round(v, 2) for v in pos_of(game)]
            game.shot(out, 'world-spawn')
    finally:
        host('command', command=f'setworldspawn {hx} {hy} {hz}')
        host('command', command='gamerule respawn_radius 10')
        for game in (host, guest):
            game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        guest.wait(20)
    for name, at in seen.items():
        assert abs(at[0] - (sx + 0.5)) < 1 and abs(at[2] - (sz + 0.5)) < 1, f'{name}: {seen}, spawn {sx} {sz}'
    return seen


def check_beds(host, guest, out):
    """A bed sets the respawn point (day or night): respawning stands the player up beside it; a bed
    that is gone, or boxed in by stone so there is nowhere to stand up, sends the player to the world
    spawn instead (#1308). The host, then the guest, each for the three beds."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='gamerule respawn_radius 0')
    host('command', command=f'setworldspawn {hx} {hy} {hz}')
    host('command', command='time set 1000')
    seen = {}
    try:
        for game, dz in ((host, -6), (guest, 6)):
            game('command', command='gamemode survival')
            x, y, z = hx + 12, hy, hz + dz
            for case in ('intact', 'gone', 'boxed'):
                game('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 3} minecraft:stone')
                game('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 3} {z + 3} minecraft:air')
                game('command', command=f'setblock {x} {y} {z} minecraft:red_bed[facing=east,part=foot]')
                game('command', command=f'setblock {x + 1} {y} {z} minecraft:red_bed[facing=east,part=head]')
                game('command', command=f'tp @s {x + 0.5} {y} {z + 1.5}')
                game.wait(20)
                game.raw('interact', x + 1, y, z)
                game.wait(10)
                # Die elsewhere, by the world spawn.
                game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 3.5}')
                game.wait(10)
                if case == 'gone':
                    game('command', command=f'fill {x} {y} {z} {x + 1} {y} {z} minecraft:air')
                elif case == 'boxed':
                    game('command', command=f'fill {x - 2} {y} {z - 2} {x + 3} {y + 2} {z + 2} minecraft:stone replace minecraft:air')
                game.wait(5)
                game('command', command='kill @s')
                game.wait(20)
                respawn_when_ready(game)
                game.wait(60)
                at = [round(v, 2) for v in pos_of(game)]
                seen[f'{game.name}-{case}'] = {'at': at, 'bed': [x + 1, y, z], 'health': own(game)['health']}
                game.shot(out, f'beds-{case}')
    finally:
        host('command', command='gamerule respawn_radius 10')
        host('command', command='time set 1000')
        for game in (host, guest):
            game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        guest.wait(20)
    for case, s in seen.items():
        (px, py, pz), (bx, by, bz) = s['at'], s['bed']
        if case.endswith('intact'):
            near = ((px - bx - 0.5) ** 2 + (pz - bz - 0.5) ** 2) ** 0.5
            assert near < 2.6 and abs(py - by) < 0.6, f'{case}: not beside its bed: {seen}'
        else:
            assert abs(px - (hx + 0.5)) < 1 and abs(pz - (hz + 0.5)) < 1, f'{case}: not at the world spawn: {seen}'
        assert s['health'] > 0, f'{case}: died again after respawning: {seen}'
    return seen


def check_death_mining(host, guest, out):
    """A player who dies while breaking obsidian, and lets go of attack on the death screen, doesn't
    break it once respawned: death forgets the digging (#1315)."""
    host('command', command='gamerule keep_inventory true')
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    try:
        for game, dz in ((host, -3), (guest, 3)):
            x, y, z = hx + 2, hy, hz + dz
            game('command', command='gamemode survival')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:diamond_pickaxe')
            game('command', command=f'setblock {x + 2} {y} {z} minecraft:obsidian')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            game.wait(10)
            game.raw('look-at', f'{x + 2.5:.2f}', f'{y + 0.5:.2f}', f'{z + 0.5:.2f}')
            game.wait(5)
            game.raw('mouse', 'left', 'down')
            game.wait(10)
            game('command', command='kill @s')
            game.wait(10)
            # The death screen lets go of the button (Minecraft.setScreen); let go again once back.
            try:
                game.raw('mouse', 'left', 'up')
            except Failed:
                pass
            game.wait(10)
            respawn_when_ready(game)
            game.wait(2)
            try:
                game.raw('mouse', 'left', 'up')
            except Failed:
                pass
            # Obsidian takes a diamond pickaxe about 190 ticks.
            game.wait(240)
            seen[game.name] = host.raw('block', x + 2, y, z).get('block')
            game.shot(out, 'death-mining')
            host('command', command=f'setblock {x + 2} {y} {z} minecraft:air')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    finally:
        host('command', command='gamerule keep_inventory false')
    for name, block in seen.items():
        assert block == 'minecraft:obsidian', f'{name}: {seen}'
    return seen


def check_absorption(host, guest, out):
    """Absorption II's golden hearts on each player's own HUD: 8 of them, and 5 once a hit of 3 took
    its share: the client shows what the server's player has (#1306)."""
    seen = {}
    for game in (host, guest):
        game('command', command='gamemode survival')
        game('command', command='effect give @s minecraft:absorption 120 1')
        game.wait(10)
        given = own(game)['absorption']
        game('command', command='damage @s 3 minecraft:generic')
        game.wait(20)
        me = own(game)
        server = server_player(host, game)
        game.shot(out, 'absorption')
        game('command', command='effect clear @s minecraft:absorption')
        seen[game.name] = {'given': given, 'left': me['absorption'], 'health': me['health'],
                           'server': server.get('absorption')}
    for name, s in seen.items():
        assert s['given'] == 8.0 and s['left'] == 5.0 == s['server'] and s['health'] == 20.0, \
            f'{name}: {seen}'
    return seen


def check_breath(host, guest, out):
    """Underwater in a Respiration III helmet for 15 s: the enchantment makes the air last on the
    server, and each player's own bubbles follow the server's air (#1304)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='gamemode survival')
        game('command', command='item replace entity @s armor.head with '
                                'minecraft:diamond_helmet[minecraft:enchantments={"minecraft:respiration":3}]')
        game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:stone')
        game('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:water')
        game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    host.wait(300)
    for game in (host, guest):
        game.shot(out, 'breath')
        seen[game.name] = {'own': own(game)['air'], 'server': server_player(host, game).get('air')}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='item replace entity @s armor.head with minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone')
    guest.wait(40)
    # 300 ticks lose about 75 of the 300 with Respiration III, all of it without.
    for name, s in seen.items():
        assert s['server'] > 150 and abs(s['own'] - s['server']) <= 10, f'{name}: {seen}'
    return seen


def check_lava(host, guest, out):
    """(--server-thread) Standing in a settled Nether lava current with Fire Resistance: the
    Nether's FAST_LAVA pushes each player about three times as hard as the Overworld's (#1309)."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    seen = {}
    back = {g.name: pos_of(g) for g in (host, guest)}
    try:
        for game, z in ((host, 0), (guest, 6)):
            game('command', command=f'execute in minecraft:the_nether run tp @s 0.5 100 {z + 0.5}')
            state = wait_dimension(game, 'minecraft:the_nether', 600)
            assert state.get('dimension') == 'minecraft:the_nether', f'{game.name} is in {state.get("dimension")}'
            game.wait(40)
            # A one-wide obsidian channel along x: a source at x = -3, the current flowing east.
            game('command', command=f'fill -5 98 {z - 2} 6 104 {z + 2} minecraft:air')
            game('command', command=f'fill -5 98 {z - 1} 6 98 {z + 1} minecraft:obsidian')
            game('command', command=f'fill -5 99 {z - 1} 6 99 {z - 1} minecraft:obsidian')
            game('command', command=f'fill -5 99 {z + 1} 6 99 {z + 1} minecraft:obsidian')
            game('command', command=f'fill -4 99 {z} -4 99 {z} minecraft:obsidian')
            game('command', command='effect give @s minecraft:fire_resistance 120 0 true')
            game('command', command='gamemode survival')
            game('command', command=f'setblock -3 99 {z} minecraft:lava')
        host.wait(100)
        for game, z in ((host, 0), (guest, 6)):
            game('command', command=f'tp @s 0.5 99 {z + 0.5}')
        host.wait(60)
        for game, z in ((host, 0), (guest, 6)):
            v = own(game)['velocity']
            seen[game.name] = {'vx': v[0], 'level': game.raw('block', 0, 99, z).get('state')}
            game.shot(out, 'lava')
    finally:
        for game, z in ((host, 0), (guest, 6)):
            host('command', command=f'execute in minecraft:the_nether run fill -5 98 {z - 2} 6 104 {z + 2} minecraft:air')
            x, y, zz = back[game.name]
            game('command', command='effect clear @s minecraft:fire_resistance')
            game('command', command=f'execute in minecraft:overworld run tp @s {x:.2f} {y:.2f} {zz:.2f}')
        for game in (host, guest):
            wait_dimension(game, 'minecraft:overworld', 600)
        guest.wait(40)
    # The Overworld's current settles at about 0.0047 a tick, the Nether's at about 0.014.
    for name, s in seen.items():
        assert s['vx'] > 0.009, f'{name}: {seen}'
    return seen


def check_ladder(host, guest, out):
    """#1255: climbing down a ladder doesn't hurt where the guest lands."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    lx, lz = x + 6, z
    guest('command', command='gamemode survival')
    guest('command', command=f'fill {lx - 2} {y} {lz - 2} {lx + 2} {y + 12} {lz + 2} minecraft:air')
    guest('command', command=f'fill {lx - 2} {y - 1} {lz - 2} {lx + 2} {y - 1} {lz + 2} minecraft:stone')
    guest('command', command=f'fill {lx} {y} {lz + 1} {lx} {y + 10} {lz + 1} minecraft:stone')
    guest('command', command=f'fill {lx} {y} {lz} {lx} {y + 10} {lz} minecraft:ladder[facing=north]')
    guest.wait(10)
    try:
        before = remote(guest)['health']
        # No input on a ladder: the guest slides down at 0.15 a tick (LivingEntity.handleOnClimbable).
        guest('command', command=f'tp @s {lx + 0.5} {y + 10} {lz + 0.4} 0 0')
        for _ in range(40):
            guest.wait(5)
            if pos_of(guest)[1] < y + 0.01:
                break
        guest.wait(20)
        landed, after = pos_of(guest), remote(guest)['health']
        assert landed[1] < y + 0.01, f'the guest stayed on the ladder at {landed}'
        assert after >= before, f'climbing down ten blocks of ladder took the guest from {before} to {after} health'
        return {'landed': landed, 'health': after}
    finally:
        guest('command', command=f'fill {lx} {y} {lz} {lx} {y + 10} {lz + 1} minecraft:air')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')


def apart(host, guest):
    """The host where it stands and the guest 12 blocks east, each on a stone floor with room
    around: neither stands in the other's way (a ray from inside a player's box hits that player)."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    spots = {}
    for label, player, dx in (('host', host, 0), ('guest', guest, 12)):
        px = x + dx
        host('command', command=f'fill {px - 3} {y - 1} {z - 3} {px + 4} {y - 1} {z + 6} minecraft:stone')
        host('command', command=f'fill {px - 3} {y} {z - 3} {px + 4} {y + 3} {z + 6} minecraft:air')
        player('command', command=f'tp @s {px + 0.5} {y} {z + 0.5} 0 0')
        spots[label] = (px, y, z)
    host.wait(10)
    guest.wait(10)
    return spots


def board(player, kind, at):
    """Right-click the `kind` object near `at` and check `player` rides it."""
    found = [o for o in player('status')['objects'] if o['kind'].endswith(kind)
             and abs(o['position'][0] - at[0]) < 2 and abs(o['position'][2] - at[2]) < 2]
    assert found, f'no {kind} near {at}'
    vehicle = found[0]
    vp = vehicle['position']
    player.raw('look-at', f'{vp[0]:.2f}', f'{vp[1] + 0.3:.2f}', f'{vp[2]:.2f}')
    player.raw('mouse', 'right', 'click')
    player.wait(10)
    riding = player('status').get('riding')
    assert riding == vehicle['id'], f'right-clicking the {kind} {vehicle["id"]}: riding {riding}'
    return vehicle


def leave_vehicle(player, kind):
    player.raw('key', 'shift', 'down')
    player.wait(10)
    player.raw('key', 'shift', 'up')
    player.wait(10)
    riding = player('status').get('riding')
    assert riding is None, f'sneaking out of the {kind}: riding {riding}'


def break_vehicle(player, vehicle_id, kind):
    """Hit it with fists until it breaks: VehicleEntity.tick takes 1 off its damage every tick."""
    for _ in range(10):
        here = [o for o in player('status')['objects'] if o['id'] == vehicle_id]
        if not here:
            return
        vp = here[0]['position']
        player.raw('look-at', f'{vp[0]:.2f}', f'{vp[1] + 0.3:.2f}', f'{vp[2]:.2f}')
        player.raw('mouse', 'left', 'click')
        player.wait(2)
    raise AssertionError(f'ten hits left the {kind} in place')


def check_boats(host, guest, out):
    """#1222: the host (single player, on its server thread or not) and the guest each board a boat
    on water with a right click, paddle it forward (the other side sees it move), sneak out, and
    break it with their fists."""
    result = {}
    spots = apart(host, guest)
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        bx = x + 3
        player('command', command='gamemode survival')
        # A pool ten blocks long in a stone basin (the world may float), with a path beside it to
        # walk back to the boat.
        player('command', command=f'fill {bx - 3} {y} {z - 2} {bx + 2} {y + 3} {z + 10} minecraft:air')
        player('command', command=f'fill {bx - 3} {y - 2} {z - 2} {bx + 2} {y - 1} {z + 10} minecraft:stone')
        player('command', command=f'fill {bx - 1} {y - 1} {z - 1} {bx + 1} {y - 1} {z + 9} minecraft:water')
        player('command', command=f'summon minecraft:oak_boat {bx + 0.5} {y - 0.1} {z + 0.5}')
        player.wait(30)
        try:
            boat = board(player, 'oak_boat', (bx + 0.5, y, z + 0.5))
            start = boat['position']
            # The summoned boat faces south: forward paddles it along the pool.
            player.raw('key', 'w', 'down')
            player.wait(30)
            player.raw('key', 'w', 'up')
            player.wait(20)
            mine = [o for o in player('status')['objects'] if o['id'] == boat['id']]
            assert mine, f'the {label} lost its boat'
            moved = math.dist(start, mine[0]['position'])
            assert moved > 0.5, f'paddling forward moved the {label}\'s boat {moved:.2f} blocks'
            # The other side sees the boat where its rider took it (the server accepted its moves).
            theirs = [o for o in objects_near(other, mine[0]['position'], 'objects')
                      if o['kind'].endswith('oak_boat')]
            assert theirs, f'the other side sees no boat near the {label}\'s'
            far = min(math.dist(o['position'], mine[0]['position']) for o in theirs)
            assert far < 1.0, f'the other side sees the {label}\'s boat {far:.2f} blocks off'
            leave_vehicle(player, 'boat')
            player('command', command=f'tp @s {bx - 1.5} {y} {mine[0]["position"][2]:.2f}')
            player.wait(5)
            break_vehicle(player, boat['id'], 'boat')
            result[label] = {'boat': boat['id'], 'paddled': round(moved, 2), 'broken': True}
        finally:
            player('command', command=f'kill @e[type=minecraft:oak_boat,x={bx},y={y},z={z + 4},distance=..10]')
            player('command', command=f'fill {bx - 1} {y - 1} {z - 1} {bx + 1} {y - 1} {z + 9} minecraft:stone')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            player.wait(5)
    return result


def check_minecarts(host, guest, out):
    """#1222: the host and the guest each get into a minecart on rails with a right click, sneak out
    beside it, and break it with their fists."""
    result = {}
    spots = apart(host, guest)
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        cx = x + 3
        player('command', command='gamemode survival')
        player('command', command=f'fill {cx - 2} {y} {z - 2} {cx + 2} {y + 3} {z + 6} minecraft:air')
        player('command', command=f'fill {cx - 2} {y - 1} {z - 2} {cx + 2} {y - 1} {z + 6} minecraft:stone')
        player('command', command=f'fill {cx} {y} {z} {cx} {y} {z + 4} minecraft:rail')
        player('command', command=f'summon minecraft:minecart {cx + 0.5} {y} {z + 0.5}')
        player.wait(30)
        try:
            cart = board(player, 'minecart', (cx + 0.5, y, z + 0.5))
            seated = pos_of(player)
            assert abs(seated[0] - (cx + 0.5)) < 0.6, f'the {label} sits at {seated}, not in the cart'
            leave_vehicle(player, 'minecart')
            out_at = pos_of(player)
            assert math.dist(out_at, seated) > 0.3, f'the {label} stayed in its seat at {out_at}'
            player.wait(5)
            break_vehicle(player, cart['id'], 'minecart')
            result[label] = {'cart': cart['id'], 'out_at': [round(v, 2) for v in out_at], 'broken': True}
        finally:
            player('command', command=f'kill @e[type=minecraft:minecart,x={cx},y={y},z={z},distance=..8]')
            player('command', command=f'kill @e[type=minecraft:item,x={cx},y={y},z={z},distance=..8]')
            player('command', command=f'fill {cx} {y} {z} {cx} {y} {z + 4} minecraft:air')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            player.wait(5)
    return result


def check_frames(host, guest, out):
    """#1222: the host and the guest each put a diamond in an item frame and turn it, and put a
    helmet on an armor stand and take it back, with right clicks."""
    result = {}
    spots = apart(host, guest)
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        # Within a survival player's 3 blocks of reach: a frame on a wall, an armor stand beside it.
        player('command', command=f'fill {x - 2} {y} {z + 1} {x + 2} {y + 2} {z + 2} minecraft:air')
        player('command', command=f'fill {x - 2} {y} {z + 3} {x + 2} {y + 2} {z + 3} minecraft:stone')
        player('command', command=f'summon minecraft:item_frame {x + 0.5} {y + 1.5} {z + 2.97} '
               f'{{block_pos:[I;{x},{y + 1},{z + 2}],Facing:2b}}')
        player('command', command=f'summon minecraft:armor_stand {x - 1.5} {y} {z + 1.5}')
        player('command', command='clear @s')
        player('command', command='item replace entity @s weapon.mainhand with minecraft:diamond')
        player.wait(30)
        try:
            frame = [o for o in player('status')['objects'] if o['kind'].endswith('item_frame')
                     and o['position'] == [x, y + 1, z + 2]]
            assert frame, f'the {label} sees no item frame at {[x, y + 1, z + 2]}'
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.5:.2f}', f'{z + 2.95:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            frame = [o for o in player('status')['objects'] if o['id'] == frame[0]['id']][0]
            assert frame['item'] == 'minecraft:diamond', f'the {label}\'s frame holds {frame["item"]}'
            assert frame['rotation'] == 1, f'the {label}\'s frame turned to {frame["rotation"]}'
            theirs = [o for o in other('status')['objects'] if o['kind'].endswith('item_frame')
                      and o['position'] == [x, y + 1, z + 2]]
            assert theirs and theirs[0]['item'] == 'minecraft:diamond' and theirs[0]['rotation'] == 1, \
                f'the other side sees the {label}\'s frame as {theirs}'
            player('command', command='item replace entity @s weapon.mainhand with minecraft:iron_helmet')
            player.wait(5)
            stand = [o for o in player('status')['objects'] if o['kind'] == 'minecraft:armor_stand'
                     and abs(o['position'][0] - (x - 1.5)) < 0.5 and abs(o['position'][2] - (z + 1.5)) < 0.5]
            assert stand, f'the {label} sees no armor stand'
            player.raw('look-at', f'{x - 1.5:.2f}', f'{y + 1.7:.2f}', f'{z + 1.5:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            worn = [o for o in player('status')['objects'] if o['id'] == stand[0]['id']][0]['equipment']
            assert 'minecraft:iron_helmet' in worn, f'the {label}\'s armor stand wears {worn}'
            player.raw('look-at', f'{x - 1.5:.2f}', f'{y + 1.7:.2f}', f'{z + 1.5:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            worn = [o for o in player('status')['objects'] if o['id'] == stand[0]['id']][0]['equipment']
            assert 'minecraft:iron_helmet' not in worn, f'taking the helmet back left {worn}'
            result[label] = {'frame': 'diamond, turned', 'stand': 'helmet on and off'}
        finally:
            player('command', command=f'kill @e[type=!minecraft:player,x={x - 3},y={y - 1},z={z},dx=6,dy=4,dz=6]')
            player('command', command=f'fill {x - 2} {y} {z + 3} {x + 2} {y + 2} {z + 3} minecraft:air')
            player('command', command='clear @s')
    return result


# Animals a player rides: (kind, summon NBT, what readies it, what is held to ride, the test
# while riding). `move` walks forward, `jump` charges the horse's jump bar, `rise` holds jump on
# a happy ghast, `sit` only rides (a llama goes where it likes).
RIDES = [
    ('horse', '{Tame:1b}', 'minecraft:saddle', None, 'move jump'),
    ('pig', '', 'minecraft:saddle', 'minecraft:carrot_on_a_stick', 'move'),
    ('strider', '', 'minecraft:saddle', 'minecraft:warped_fungus_on_a_stick', 'move'),
    ('camel', '', 'minecraft:saddle', None, 'move'),
    ('donkey', '{Tame:1b}', 'minecraft:saddle', None, 'move'),
    ('llama', '{Tame:1b}', None, None, 'sit'),
    ('happy_ghast', '', 'minecraft:white_harness', None, 'rise'),
]
HEIGHTS = {'horse': 1.6, 'donkey': 1.5, 'pig': 0.9, 'strider': 1.7, 'camel': 2.375, 'llama': 1.87,
           'happy_ghast': 4.0}


def mob_seen(game, mob_id):
    found = [m for m in game.raw('mobs', 96).get('near', []) if m['id'] == mob_id]
    return found[0] if found else None


def hold(game, key, ticks):
    game.raw('key', key, 'down')
    game.wait(ticks)
    game.raw('key', key, 'up')


def ride(player, other, label, spot, kind, nbt, ready, stick, test):
    """Summon `kind` in front of `player`, ready it (saddle, harness), get on with a right click,
    run `test`, and get off by sneaking. The other side sees the animal move."""
    x, y, z = spot
    # In reach (3 blocks), clear of the player: a happy ghast is 4 blocks wide.
    at = (x + 0.5, z + {'happy_ghast': 3.2, 'camel': 3.0}.get(kind, 2.5))
    # The first item in hand before it appears: a happy ghast drifts out of reach quickly.
    player('command', command=f'item replace entity @s weapon.mainhand with {ready or stick or "minecraft:air"}')
    player('command', command=f'summon minecraft:{kind} {at[0]} {y} {at[1]} {nbt}')
    player.wait(3)
    found = [m for m in player.raw('mobs', 16).get('near', []) if m['kind'].endswith(kind) and m['health'] > 0]
    assert found, f'the {label} sees no {kind}'
    mob = found[0]
    def click(item):
        player('command', command=f'item replace entity @s weapon.mainhand with {item or "minecraft:air"}')
        player.wait(2)
        m = mob_seen(player, mob['id'])
        assert m, f'the {label} lost its {kind}'
        p = m['pos']
        # Back in front of it if it wandered off (a happy ghast drifts away fast).
        if abs(p[0] - (x + 0.5)) > 0.5 or abs(p[2] - at[1]) > 0.5:
            player('command', command=f'tp @s {p[0]:.2f} {y} {p[2] - (at[1] - z - 0.5):.2f} 0 0')
            player.wait(1)
        player.raw('look-at', f'{p[0]:.2f}', f'{p[1] + HEIGHTS[kind] / 2:.2f}', f'{p[2]:.2f}')
        player.raw('mouse', 'right', 'click')
        player.wait(5)
    try:
        if ready:
            click(ready)
        click(stick)
        riding = player('status').get('riding')
        assert riding == mob['id'], f'the {label} right-clicked the {kind} {mob["id"]}: riding {riding}'
        result = {'rode': True}
        # South, along the cleared floor.
        player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.6:.2f}', f'{z + 40.5:.2f}')
        player.wait(5)
        start = mob_seen(player, mob['id'])['pos']
        if 'move' in test:
            hold(player, 'w', 60)
            player.wait(5)
            mine = mob_seen(player, mob['id'])['pos']
            theirs = mob_seen(other, mob['id'])
            moved = math.dist(start, mine)
            assert moved > 1.0, f"the {label}'s {kind} moved {moved:.2f} blocks in 3 s of forward"
            assert theirs and math.dist(theirs['pos'], mine) < 1.5, \
                f"the other side sees the {label}'s {kind} at {theirs and theirs['pos']}, not {mine}"
            seated = pos_of(player)
            assert math.dist(seated[::2], mine[::2]) < 1.5, f'the {label} sits at {seated}, its {kind} at {mine}'
            result['moved'] = round(moved, 2)
        if 'jump' in test:
            base = mob_seen(player, mob['id'])['pos'][1]
            player.raw('key', 'space', 'down')
            player.wait(10)
            player.raw('key', 'space', 'up')
            top = base
            for _ in range(10):
                player.wait(2)
                top = max(top, mob_seen(player, mob['id'])['pos'][1])
            assert top - base > 0.4, f"the {label}'s {kind} jumped {top - base:.2f} blocks"
            result['jumped'] = round(top - base, 2)
        if 'rise' in test:
            base = mob_seen(player, mob['id'])['pos'][1]
            hold(player, 'space', 40)
            player.wait(5)
            rose = mob_seen(player, mob['id'])['pos'][1] - base
            assert rose > 0.5, f"the {label}'s {kind} rose {rose:.2f} blocks while it held jump"
            result['rose'] = round(rose, 2)
        hold(player, 'shift', 10)
        player.wait(10)
        riding = player('status').get('riding')
        assert riding is None, f'sneaking off the {kind}: the {label} still rides {riding}'
        return result
    finally:
        player('command', command=f'kill @e[type=minecraft:{kind},x={x},y={y},z={z},distance=..48]')
        player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
        player.wait(5)


def check_riding(host, guest, out):
    """The host (single player, on its server thread or not) and the guest each saddle and ride a
    horse (walk and jump), a pig and a strider (with their sticks), a camel and a donkey, sit on a
    llama, and harness and fly a happy ghast; they get off by sneaking."""
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        player('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 40} minecraft:stone')
        player('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 12} {z + 40} minecraft:air')
        player.wait(10)
        result[label] = {}
        # LAN_E2E_RIDES=pig,horse picks some of them.
        only = [k for k in os.environ.get('LAN_E2E_RIDES', '').split(',') if k]
        for kind, nbt, ready, stick, test in (r for r in RIDES if not only or r[0] in only):
            result[label][kind] = ride(player, other, label, (x, y, z), kind, nbt, ready, stick, test)
        player('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    return result


# Mount inventories: (kind, summon NBT, what goes in by shift-click and the menu slot it lands
# in: 700 the saddle, 701 the body armour or carpet, 710 the chest's first slot).
MOUNT_MENUS = [
    ('horse', '{Tame:1b,NoAI:1b}', [('minecraft:saddle', 700), ('minecraft:iron_horse_armor', 701)]),
    ('donkey', '{Tame:1b,NoAI:1b,ChestedHorse:1b}', [('minecraft:saddle', 700), ('minecraft:dirt', 710)]),
    ('llama', '{Tame:1b,NoAI:1b,ChestedHorse:1b,Strength:3}',
     [('minecraft:red_carpet', 701), ('minecraft:dirt', 710)]),
]


def mount_screen(game, ticks=20):
    """The open mount menu's slots and items ({700: 'minecraft:saddle', ...}), waiting up to
    `ticks` for it to open; None when no mount menu opened."""
    for _ in range(ticks // 2):
        slots = {}
        for w in game.raw('ui').get('widgets', []):
            wid = w.get('id', '')
            if wid.startswith('slot-') and wid[5:].isdigit() and int(wid[5:]) >= 700:
                slots[int(wid[5:])] = (w.get('stack') or {}).get('item')
        if slots:
            return slots
        game.wait(2)
    return None


def check_mounts(host, guest, out):
    """The host and the guest each open a tamed horse's, donkey's and llama's inventory with
    shift + right click (`ClientboundMountScreenOpenPacket` for a server's player), put on a
    saddle, horse armour or a carpet and fill the chest by shift-clicking, close it, then ride the
    mount and open its inventory with the inventory key (OPEN_INVENTORY): it shows what was put
    in, and the server took the items from the inventory."""
    spots = apart(host, guest)
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        result[label] = {}
        for kind, nbt, puts in MOUNT_MENUS:
            player('command', command='clear @s')
            for item, _ in puts:
                player('command', command=f'give @s {item}')
            player('command', command=f'summon minecraft:{kind} {x + 0.5} {y} {z + 2.5} {nbt}')
            player.wait(5)
            found = [m for m in player.raw('mobs', 16).get('near', [])
                     if m['kind'].endswith(kind) and m['health'] > 0]
            assert found, f'the {label} sees no {kind}'
            mob = found[0]
            p = mob['pos']
            aim = (f'{p[0]:.2f}', f'{p[1] + HEIGHTS[kind] / 2:.2f}', f'{p[2]:.2f}')
            try:
                player.raw('look-at', *aim)
                player.raw('key', 'shift', 'down')
                player.wait(3)
                player.raw('mouse', 'right', 'click')
                player.wait(5)
                player.raw('key', 'shift', 'up')
                slots = mount_screen(player)
                assert slots is not None, f"the {label}'s shift + right click on the {kind} opened no inventory"
                for item, _ in puts:
                    player.raw('click-slot', hotbar_slot(player, item), 'shift')
                    player.wait(5)
                slots = mount_screen(player)
                for item, slot in puts:
                    assert slots.get(slot) == item, f"the {label}'s {kind}: {item} shift-clicked, slot {slot} holds {slots.get(slot)}"
                player.raw('key', 'esc', 'tap')
                player.wait(5)
                for item, _ in puts:
                    left = server_count(player, item.split(':')[1])
                    assert left == 0, f'the server still counts {left} {item} on the {label}'
                player.raw('look-at', *aim)
                player.raw('mouse', 'right', 'click')
                player.wait(8)
                riding = player('status').get('riding')
                assert riding == mob['id'], f'the {label} right-clicked its {kind}: riding {riding}'
                player.raw('key', 'e', 'tap')
                again = mount_screen(player)
                assert again is not None, f'the inventory key on the {label}\'s {kind} opened no inventory'
                for item, slot in puts:
                    assert again.get(slot) == item, f"the {label}'s {kind} reopened: slot {slot} holds {again.get(slot)}, not {item}"
                player.raw('key', 'esc', 'tap')
                player.wait(3)
                hold(player, 'shift', 10)
                player.wait(5)
                result[label][kind] = {slot: item for item, slot in puts}
            finally:
                back_to_game(player)
                player('command', command=f'kill @e[type=minecraft:{kind},x={x},y={y},z={z},distance=..16]')
                player.wait(2)
                player('command', command=f'kill @e[type=minecraft:item,x={x},y={y},z={z},distance=..16]')
                player('command', command='clear @s')
                player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
                player.wait(5)
    return result


def check_lightning(host, guest, out):
    """A bolt summoned beside each player: both sides get the bolt entity and flash the sky (the
    host's window too on its server thread, FEATURES 4z(f))."""
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = (int(v // 1) for v in pos_of(player))
        before = {g.name: g('status')['lightning'] for g in (host, guest)}
        # No fire where it strikes: on stone with nothing around.
        player('command', command=f'fill {x + 4} {y - 1} {z - 2} {x + 8} {y - 1} {z + 2} minecraft:stone')
        player('command', command=f'fill {x + 4} {y} {z - 2} {x + 8} {y + 3} {z + 2} minecraft:air')
        player('command', command=f'summon minecraft:lightning_bolt {x + 6.5} {y} {z + 0.5}')
        seen = {}
        for _ in range(40):
            player.wait(1)
            seen = {g.name: g('status')['lightning'] for g in (host, guest)}
            if all(seen[n]['struck'] > before[n]['struck'] and seen[n]['flashes'] > before[n]['flashes']
                   for n in seen):
                break
        player('command', command=f'fill {x + 4} {y} {z - 2} {x + 8} {y + 3} {z + 2} minecraft:air')
        for n in seen:
            assert seen[n]['struck'] > before[n]['struck'], f"{label}'s bolt: the {n} got no bolt ({seen[n]})"
            assert seen[n]['flashes'] > before[n]['flashes'], f"{label}'s bolt: the {n}'s sky didn't flash ({seen[n]})"
        result[label] = {n: {'bolts': seen[n]['struck'] - before[n]['struck'],
                             'flashes': seen[n]['flashes'] - before[n]['flashes']} for n in seen}
    return result


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


CHECK_FNS = {'tab': check_tab, 'self': check_self, 'armour': check_armour,
             'nametags': check_nametags, 'chest': check_chest, 'crafting': check_crafting,
             'sign': check_sign, 'damage': check_damage, 'mobs': check_mobs,
             'hazards': check_hazards, 'pvp': check_pvp, 'respawn': check_respawn,
             'containers': check_containers, 'feedback': check_feedback, 'titles': check_titles,
             'chat': check_chat, 'machines': check_machines, 'brewing': check_brewing,
             'loom': check_loom, 'stonecutter': check_stonecutter, 'grindstone': check_grindstone,
             'anvil': check_anvil, 'smithing': check_smithing, 'enchanting': check_enchanting,
             'beacon': check_beacon, 'crafter': check_crafter, 'lectern': check_lectern,
             'cartography': check_cartography, 'hurt': check_hurt, 'commands': check_commands,
             'recipes': check_recipes, 'trials': check_trials, 'walls': check_walls,
             'effects': check_effects, 'portals': check_portals, 'distance': check_distance,
             'clicks': check_clicks, 'sleep': check_sleep, 'push': check_push,
             'eat': check_eat, 'eating': check_eating, 'xp': check_xp,
             'objects': check_objects, 'projectiles': check_projectiles, 'blast': check_blast,
             'light': check_light, 'plates': check_plates, 'swings': check_swings,
             'cracks': check_cracks, 'time': check_time, 'growth': check_growth,
             'flight': check_flight, 'fireball': check_fireball, 'deflect': check_deflect,
             'barter': check_barter, 'worn': check_worn, 'kills': check_kills,
             'arrows': check_arrows, 'predict': check_predict, 'ladder': check_ladder,
             'boats': check_boats, 'minecarts': check_minecarts, 'frames': check_frames,
             'lightning': check_lightning, 'riding': check_riding, 'mounts': check_mounts,
             'worldspawn': check_world_spawn, 'beds': check_beds, 'shield': check_shield, 'deathmining': check_death_mining,
             'contacts': check_contacts, 'absorption': check_absorption, 'breath': check_breath,
             'lava': check_lava, 'levels': check_levels, 'drops': check_drops,
             'underwater': check_underwater,
             'looks': check_looks,
             'leads': check_leads,
             'thrown': check_thrown,
             'sparks': check_sparks,
             'hud': check_hud,
             'sidebar': check_sidebar}


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
