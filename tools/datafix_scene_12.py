"""The 1.12.2 scene for tools/datafix_worlds.py: a rich sample world for the DataFixers of
1.13 (DataVersion 1343 to 1631), written in the command syntax of 1.12.

The scene is a set of 96 x 96 stone platforms ("layers", one per chunk section: y = 100, 116 .. 244)
above the spawn area. Every block family of the 1.12 id/data space sits on its own cells, one empty cell
between neighbours (so that no neighbour update rewrites a state), with the tile entities, items,
entities, player data, scoreboard, maps, advancements and statistics that the flattening touches.
States that the game recomputes at once (powered pistons, repeaters, fluids, hanging entities ...) are
placed in the very last tick, together with `save-all flush`, and the server is killed right after: the saved chunks keep them.

  populate(server, Bot, port, protocol, name, problems)  -> runs everything, bot joined, returns the bot

What a real server cannot produce is written by hand after the stop, see `fixtures()` (marked there).
"""
import json
import queue
import re
import shutil
import socket
import struct
import sys
import threading
import time
from pathlib import Path

import lab

BX, BZ, SIZE = 144, 144, 96          # south-west corner and size of the platforms (inside the spawn chunks)
LAYERS = list(range(100, 245, 16))   # platform y levels: one chunk section each, the last one reaches y = 255
UUID_HI = 0x6000


# ------------------------------------------------------------------ versions
# One builder for 1.8.9 to 1.12.2; `ERA` is the minor version (8 to 12), set by `set_protocol` from the bot's protocol.
# 1.8: numeric item ids in saves, CamelCase entity and tile entity ids, `Equipment`, `Riding`
# 1.9: `ArmorItems`/`HandItems`, `Passengers`, shulkers, lingering potions, tipped arrows, elytra, chorus, purpur ...
# 1.10: husks and strays (zombie / skeleton types), polar bears, magma, bone blocks, nether wart blocks
# 1.11: namespaced entity ids, evokers, vindicators, vexes, llamas, separate donkeys and so on, shulker boxes, observers
# 1.12: parrots, illusioners, concrete, glazed terracotta, advancements instead of achievements
ERA = 12
PROTOCOL_ERA = {47: 8, 107: 9, 108: 9, 109: 9, 110: 9, 210: 10, 315: 11, 316: 11, 335: 12, 338: 12, 340: 12}
VALID_ITEMS = None        # item names the server accepts (probed at the start), None: everything


def set_protocol(protocol):
    global ERA
    ERA = PROTOCOL_ERA[protocol]


def block_ok(name):
    """Whether this version has the block (by its numeric id)."""
    i = BLOCK_IDS.get(name)
    if i is None:
        return False
    limit = {8: 197, 9: 212, 10: 217, 11: 234, 12: 255}[ERA]
    return i <= limit or (i == 255 and ERA >= 9)

# ------------------------------------------------------------------ tables
# `id:name=data,ranges` of every 1.12 block state (from vanilla's BlockStateData), id = numeric block id.
BLOCKS = """
0:air=0 1:stone=0-6 2:grass=0 3:dirt=0-2 4:cobblestone=0 5:planks=0-5 6:sapling=0-5,8-13 7:bedrock=0
8:flowing_water=0-15 9:water=0-15 10:flowing_lava=0-15 11:lava=0-15 12:sand=0-1 13:gravel=0 14:gold_ore=0
15:iron_ore=0 16:coal_ore=0 17:log=0-15 18:leaves=0-15 19:sponge=0-1 20:glass=0 21:lapis_ore=0 22:lapis_block=0
23:dispenser=0-5,8-13 24:sandstone=0-2 25:noteblock=0 26:bed=0-3,8-15 27:golden_rail=0-5,8-13
28:detector_rail=0-5,8-13 29:sticky_piston=0-5,8-13 30:web=0 31:tallgrass=0-2 32:deadbush=0 33:piston=0-5,8-13
34:piston_head=0-5,8-13 35:wool=0-15 36:piston_extension=0-5,8-13 37:yellow_flower=0 38:red_flower=0-8
39:brown_mushroom=0 40:red_mushroom=0 41:gold_block=0 42:iron_block=0 43:double_stone_slab=0-15 44:stone_slab=0-15
45:brick_block=0 46:tnt=0-1 47:bookshelf=0 48:mossy_cobblestone=0 49:obsidian=0 50:torch=1-5 51:fire=0-15
52:mob_spawner=0 53:oak_stairs=0-7 54:chest=2-5 55:redstone_wire=0-15 56:diamond_ore=0 57:diamond_block=0
58:crafting_table=0 59:wheat=0-7 60:farmland=0-7 61:furnace=2-5 62:lit_furnace=2-5 63:standing_sign=0-15
64:wooden_door=0-11 65:ladder=2-5 66:rail=0-9 67:stone_stairs=0-7 68:wall_sign=2-5 69:lever=0-15
70:stone_pressure_plate=0-1 71:iron_door=0-11 72:wooden_pressure_plate=0-1 73:redstone_ore=0 74:lit_redstone_ore=0
75:unlit_redstone_torch=1-5 76:redstone_torch=1-5 77:stone_button=0-5,8-13 78:snow_layer=0-7 79:ice=0 80:snow=0
81:cactus=0-15 82:clay=0 83:reeds=0-15 84:jukebox=0-1 85:fence=0 86:pumpkin=0-3 87:netherrack=0 88:soul_sand=0
89:glowstone=0 90:portal=1-2 91:lit_pumpkin=0-3 92:cake=0-6 93:unpowered_repeater=0-15 94:powered_repeater=0-15
95:stained_glass=0-15 96:trapdoor=0-15 97:monster_egg=0-5 98:stonebrick=0-3 99:brown_mushroom_block=0-10,14-15
100:red_mushroom_block=0-10,14-15 101:iron_bars=0 102:glass_pane=0 103:melon_block=0 104:pumpkin_stem=0-7
105:melon_stem=0-7 106:vine=0-15 107:fence_gate=0-15 108:brick_stairs=0-7 109:stone_brick_stairs=0-7 110:mycelium=0
111:waterlily=0 112:nether_brick=0 113:nether_brick_fence=0 114:nether_brick_stairs=0-7 115:nether_wart=0-3
116:enchanting_table=0 117:brewing_stand=0-7 118:cauldron=0-3 119:end_portal=0 120:end_portal_frame=0-7
121:end_stone=0 122:dragon_egg=0 123:redstone_lamp=0 124:lit_redstone_lamp=0 125:double_wooden_slab=0-5
126:wooden_slab=0-5,8-13 127:cocoa=0-11 128:sandstone_stairs=0-7 129:emerald_ore=0 130:ender_chest=2-5
131:tripwire_hook=0-15 132:tripwire=0-1,4-5,8-9,12-13 133:emerald_block=0 134:spruce_stairs=0-7 135:birch_stairs=0-7
136:jungle_stairs=0-7 137:command_block=0-5,8-13 138:beacon=0 139:cobblestone_wall=0-1 140:flower_pot=0-15
141:carrots=0-7 142:potatoes=0-7 143:wooden_button=0-5,8-13 144:skull=0-5,8-13 145:anvil=0-11 146:trapped_chest=2-5
147:light_weighted_pressure_plate=0-15 148:heavy_weighted_pressure_plate=0-15 149:unpowered_comparator=0-15
150:powered_comparator=0-15 151:daylight_detector=0-15 152:redstone_block=0 153:quartz_ore=0 154:hopper=0,2-5,8,10-13
155:quartz_block=0-4 156:quartz_stairs=0-7 157:activator_rail=0-5,8-13 158:dropper=0-5,8-13
159:stained_hardened_clay=0-15 160:stained_glass_pane=0-15 161:leaves2=0-1,4-5,8-9,12-13 162:log2=0-1,4-5,8-9,12-13
163:acacia_stairs=0-7 164:dark_oak_stairs=0-7 165:slime=0 166:barrier=0 167:iron_trapdoor=0-15 168:prismarine=0-2
169:sea_lantern=0 170:hay_block=0,4,8 171:carpet=0-15 172:hardened_clay=0 173:coal_block=0 174:packed_ice=0
175:double_plant=0-5,8-11 176:standing_banner=0-15 177:wall_banner=2-5 178:daylight_detector_inverted=0-15
179:red_sandstone=0-2 180:red_sandstone_stairs=0-7 181:double_stone_slab2=0,8 182:stone_slab2=0,8
183:spruce_fence_gate=0-15 184:birch_fence_gate=0-15 185:jungle_fence_gate=0-15 186:dark_oak_fence_gate=0-15
187:acacia_fence_gate=0-15 188:spruce_fence=0 189:birch_fence=0 190:jungle_fence=0 191:dark_oak_fence=0
192:acacia_fence=0 193:spruce_door=0-11 194:birch_door=0-11 195:jungle_door=0-11 196:acacia_door=0-11
197:dark_oak_door=0-11 198:end_rod=0-5 199:chorus_plant=0 200:chorus_flower=0-5 201:purpur_block=0
202:purpur_pillar=0,4,8 203:purpur_stairs=0-7 204:purpur_double_slab=0 205:purpur_slab=0,8 206:end_bricks=0
207:beetroots=0-3 208:grass_path=0 209:end_gateway=0 210:repeating_command_block=0-5,8-13
211:chain_command_block=0-5,8-13 212:frosted_ice=0-3 213:magma=0 214:nether_wart_block=0 215:red_nether_brick=0
216:bone_block=0,4,8 217:structure_void=0 218:observer=0-5,8-13 219:white_shulker_box=0-5 220:orange_shulker_box=0-5
221:magenta_shulker_box=0-5 222:light_blue_shulker_box=0-5 223:yellow_shulker_box=0-5 224:lime_shulker_box=0-5
225:pink_shulker_box=0-5 226:gray_shulker_box=0-5 227:silver_shulker_box=0-5 228:cyan_shulker_box=0-5
229:purple_shulker_box=0-5 230:blue_shulker_box=0-5 231:brown_shulker_box=0-5 232:green_shulker_box=0-5
233:red_shulker_box=0-5 234:black_shulker_box=0-5 235:white_glazed_terracotta=0-3 236:orange_glazed_terracotta=0-3
237:magenta_glazed_terracotta=0-3 238:light_blue_glazed_terracotta=0-3 239:yellow_glazed_terracotta=0-3
240:lime_glazed_terracotta=0-3 241:pink_glazed_terracotta=0-3 242:gray_glazed_terracotta=0-3
243:silver_glazed_terracotta=0-3 244:cyan_glazed_terracotta=0-3 245:purple_glazed_terracotta=0-3
246:blue_glazed_terracotta=0-3 247:brown_glazed_terracotta=0-3 248:green_glazed_terracotta=0-3
249:red_glazed_terracotta=0-3 250:black_glazed_terracotta=0-3 251:concrete=0-15 252:concrete_powder=0-15
255:structure_block=0-3
"""
# `name=damage ranges` of the items whose damage value picks the flattened item (ItemStackTheFlatteningFix).
ITEM_DAMAGES = """
stone=0-6 dirt=0-2 leaves=0-3 leaves2=0-1 log=0-3 log2=0-1 sapling=0-5 planks=0-5 sand=0-1 quartz_block=0-2 anvil=0-2
wool=0-15 carpet=0-15 hardened_clay=0 stained_hardened_clay=0-15 silver_glazed_terracotta=0 stained_glass=0-15
stained_glass_pane=0-15 prismarine=0-2 concrete=0-15 concrete_powder=0-15 cobblestone_wall=0-1 sandstone=0-2
red_sandstone=0-2 stonebrick=0-3 monster_egg=0-5 yellow_flower=0 red_flower=0-8 double_plant=0-5 deadbush=0
tallgrass=0-2 sponge=0-1 purpur_slab=0 stone_slab=0-7 stone_slab2=0 wooden_slab=0-5 coal=0-1 fish=0-3 cooked_fish=0-1
skull=0-5 golden_apple=0-1 fireworks=0 firework_charge=0 dye=0-15 silver_shulker_box=0 fence=0 fence_gate=0
wooden_door=0 boat=0 lit_pumpkin=0 pumpkin=0 trapdoor=0 nether_brick=0 red_nether_brick=0 netherbrick=0
wooden_button=0 wooden_pressure_plate=0 noteblock=0 bed=0-15 banner=0-15 grass=0 brick_block=0 end_bricks=0
golden_rail=0 magma=0 quartz_ore=0 reeds=0 slime=0 stone_stairs=0 waterlily=0 web=0 snow=0 snow_layer=0 record_11=0
record_13=0 record_blocks=0 record_cat=0 record_chirp=0 record_far=0 record_mall=0 record_mellohi=0 record_stal=0
record_strad=0 record_wait=0 record_ward=0
"""
# Items a 1.12.2 server accepts (numeric ids 0..422 plus later ones).
ITEM_NAMES = """
stone grass dirt cobblestone planks sapling bedrock sand gravel gold_ore iron_ore coal_ore log leaves sponge glass
lapis_ore lapis_block dispenser sandstone noteblock golden_rail detector_rail sticky_piston web tallgrass deadbush
piston wool yellow_flower red_flower brown_mushroom red_mushroom gold_block iron_block stone_slab brick_block tnt
bookshelf mossy_cobblestone obsidian torch mob_spawner oak_stairs chest diamond_ore diamond_block crafting_table
farmland furnace ladder rail stone_stairs lever stone_pressure_plate wooden_pressure_plate redstone_ore redstone_torch
stone_button snow_layer ice snow cactus clay jukebox fence pumpkin netherrack soul_sand glowstone lit_pumpkin
stained_glass trapdoor monster_egg stonebrick brown_mushroom_block red_mushroom_block iron_bars glass_pane melon_block
vine fence_gate brick_stairs stone_brick_stairs mycelium waterlily nether_brick nether_brick_fence nether_brick_stairs
enchanting_table end_portal_frame end_stone dragon_egg redstone_lamp wooden_slab sandstone_stairs emerald_ore
ender_chest tripwire_hook emerald_block spruce_stairs birch_stairs jungle_stairs command_block beacon cobblestone_wall
wooden_button anvil trapped_chest light_weighted_pressure_plate heavy_weighted_pressure_plate daylight_detector
redstone_block quartz_ore hopper quartz_block quartz_stairs activator_rail dropper stained_hardened_clay
stained_glass_pane leaves2 log2 acacia_stairs dark_oak_stairs hay_block carpet hardened_clay coal_block packed_ice
double_plant iron_shovel iron_pickaxe iron_axe flint_and_steel apple bow arrow coal diamond iron_ingot gold_ingot
iron_sword wooden_sword wooden_shovel wooden_pickaxe wooden_axe stone_sword stone_shovel stone_pickaxe stone_axe
diamond_sword diamond_shovel diamond_pickaxe diamond_axe stick bowl mushroom_stew golden_sword golden_shovel
golden_pickaxe golden_axe string feather gunpowder wooden_hoe stone_hoe iron_hoe diamond_hoe golden_hoe wheat_seeds
wheat bread leather_helmet leather_chestplate leather_leggings leather_boots chainmail_helmet chainmail_chestplate
chainmail_leggings chainmail_boots iron_helmet iron_chestplate iron_leggings iron_boots diamond_helmet
diamond_chestplate diamond_leggings diamond_boots golden_helmet golden_chestplate golden_leggings golden_boots flint
porkchop cooked_porkchop painting golden_apple sign wooden_door bucket water_bucket lava_bucket minecart saddle
iron_door redstone snowball boat leather milk_bucket brick clay_ball reeds paper book slime_ball chest_minecart
furnace_minecart egg compass fishing_rod clock glowstone_dust fish dye bone sugar cake bed repeater cookie filled_map
shears melon pumpkin_seeds melon_seeds beef cooked_beef chicken cooked_chicken rotten_flesh ender_pearl blaze_rod
ghast_tear gold_nugget nether_wart potion glass_bottle spider_eye fermented_spider_eye blaze_powder magma_cream
brewing_stand cauldron ender_eye speckled_melon spawn_egg experience_bottle fire_charge writable_book written_book
emerald item_frame flower_pot carrot potato baked_potato poisonous_potato map golden_carrot skull carrot_on_a_stick
nether_star pumpkin_pie fireworks firework_charge enchanted_book comparator netherbrick quartz tnt_minecart
hopper_minecart iron_horse_armor golden_horse_armor diamond_horse_armor lead name_tag command_block_minecart record_13
record_cat record_blocks record_chirp record_far record_mall record_mellohi record_stal record_strad record_ward
record_11 record_wait cooked_fish shield elytra totem_of_undying spectral_arrow tipped_arrow splash_potion
lingering_potion beetroot beetroot_seeds beetroot_soup dragon_breath chorus_fruit chorus_flower end_crystal
shulker_shell iron_nugget knowledge_book spruce_boat birch_boat jungle_boat acacia_boat dark_oak_boat mutton
cooked_mutton rabbit cooked_rabbit rabbit_stew rabbit_foot rabbit_hide prismarine_shard prismarine_crystals
purpur_block end_rod structure_block structure_void barrier grass_path observer white_shulker_box silver_shulker_box
white_glazed_terracotta concrete concrete_powder bone_block magma nether_wart_block red_nether_brick
repeating_command_block chain_command_block sea_lantern prismarine red_sandstone red_sandstone_stairs purpur_stairs
purpur_slab purpur_pillar end_bricks spruce_door birch_door jungle_door acacia_door dark_oak_door spruce_fence_gate
spruce_fence stone_slab2 banner iron_trapdoor
"""


def parse_ranges(text):
    out = []
    for part in text.split(','):
        a, _, b = part.partition('-')
        out.extend(range(int(a), int(b or a) + 1))
    return out


BLOCK_STATES = {}   # name -> list of data values
BLOCK_IDS = {}      # name -> numeric id
for _entry in BLOCKS.split():
    _id, _, _rest = _entry.partition(':')
    _name, _, _data = _rest.partition('=')
    BLOCK_IDS[_name] = int(_id)
    BLOCK_STATES[_name] = parse_ranges(_data)
ITEM_VARIANTS = {}
for _entry in ITEM_DAMAGES.split():
    _name, _, _data = _entry.partition('=')
    ITEM_VARIANTS[_name] = parse_ranges(_data)
ITEMS = ITEM_NAMES.split()

COLORS = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'silver', 'cyan', 'purple',
          'blue', 'brown', 'green', 'red', 'black']
WOODS = ['oak', 'spruce', 'birch', 'jungle', 'acacia', 'dark_oak']


# ------------------------------------------------------------------ NBT text helpers
def q(s):
    """A quoted NBT string."""
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def js(obj):
    """A JSON text as an NBT string (1.12 stores text components as JSON strings)."""
    if isinstance(obj, str) and ERA < 12:
        obj = {'text': obj}         # before 1.12 a JSON string primitive makes the command fail
    return q(json.dumps(obj, separators=(',', ':')))


def full(name):
    return name if ':' in name else 'minecraft:' + name


class Stack(str):
    """An item compound that remembers its parts (for `replaceitem` / `give`)."""

    def __new__(cls, text, name, count, damage, tag, numeric):
        obj = str.__new__(cls, text)
        obj.name, obj.count, obj.damage, obj.tag, obj.numeric = name, count, damage, tag, numeric
        return obj

    def command_args(self):
        return f'{full(self.name)} {self.count} {self.damage}' + (f' {self.tag}' if self.tag else '')


def item(name, count=1, damage=0, tag=None, slot=None, numeric=None):
    """An item stack compound. `numeric`: write the 1.7 style numeric `id:Ns` instead of the name."""
    if VALID_ITEMS is not None and numeric is None and name not in VALID_ITEMS:
        name, tag = 'stick', None      # an item this version lacks
    parts = []
    if slot is not None:
        parts.append(f'Slot:{slot}b')
    parts.append(f'id:{numeric}s' if numeric is not None else f'id:{q(full(name))}')
    parts.append(f'Count:{count}b')
    parts.append(f'Damage:{damage}s')
    if tag:
        parts.append(f'tag:{tag}')
    return Stack('{' + ','.join(parts) + '}', name, count, damage, tag, numeric)


def slotted(stacks, first=0):
    """Put `Slot` into the stack compounds (strings starting with `{`)."""
    return '[' + ','.join('{Slot:%db,%s' % (first + i, s[1:]) for i, s in enumerate(stacks)) + ']'


def ench(*pairs):
    """`ench:[{id:16s,lvl:3s}]`: 1.12 enchantments are numeric ids."""
    return 'ench:[' + ','.join('{id:%ds,lvl:%ds}' % p for p in pairs) + ']'


def stored_ench(*pairs):
    return 'StoredEnchantments:[' + ','.join('{id:%ds,lvl:%ds}' % p for p in pairs) + ']'


def display(name=None, lore=None, color=None):
    parts = []
    if name is not None:
        parts.append(f'Name:{q(name)}')
    if lore:
        parts.append('Lore:[' + ','.join(q(l) for l in lore) + ']')
    if color is not None:
        parts.append(f'color:{color}')
    return 'display:{' + ','.join(parts) + '}'


# ------------------------------------------------------------------ running commands in batches
FAIL = re.compile(r"(couldn't|cannot|can't|unknown|usage:|invalid|no such|not found|outside of the world|"
                  r"exception|expected|failed|unable|could not|does not exist|there is no|not a valid|"
                  r"incorrect|malformed|error|no entity|must be|too (big|large|many|long)|not allowed|"
                  r"not an? |no (item|block|player|entities))", re.I)
FINE = re.compile(r"(already have it|Can't keep up|No blocks filled|logged in with entity id|lost connection|left the game|joined the game)")


def clean(line):
    return re.sub(r'^\[[0-9:]+\] \[[^\]]*\]: ', '', line)


def batch(server, commands, problems, chunk=120, quiet=False):
    """Run the commands one after the other (many per tick), return their output lines. Failures go to `problems`."""
    outputs = []
    for start in range(0, len(commands), chunk):
        part = commands[start:start + chunk]
        marks = []
        text = []
        for c in part:
            server.counter += 1
            mark = f'@@{server.counter}@@'
            marks.append(mark)
            text.append(c)
            text.append('say ' + mark)
        server.process.stdin.write('\n'.join(text) + '\n')
        server.process.stdin.flush()
        i = 0
        current = []
        deadline = time.time() + 600
        while i < len(part):
            try:
                line = clean(server.queue.get(timeout=5))
            except queue.Empty:
                if server.process.poll() is not None or time.time() > deadline:
                    raise TimeoutError(f'server did not answer {part[i][:100]!r}')
                continue
            if marks[i] in line:
                outputs.append(current)
                bad = [l for l in current if FAIL.search(l) and not FINE.search(l)]
                if bad and not quiet:
                    problems.append((part[i][:200], bad[:2]))
                current = []
                i += 1
            else:
                current.append(line)
    return outputs


# ------------------------------------------------------------------ layout
class Layers:
    """Platforms are created on first use; rows of them are handed out to `Packer`s."""

    def __init__(self, scene):
        self.scene = scene
        self.next = 0
        self.made = set()
        self.y = None
        self.z = 0
        self.n = 0

    def take(self):
        """A whole new platform."""
        if self.next >= len(LAYERS):
            raise RuntimeError('out of layers')
        y = LAYERS[self.next]
        self.next += 1
        self.platform(y)
        return y

    def fresh(self):
        """The next row goes on a new platform (so that the platform is used by one group of things only)."""
        self.y = None

    def row(self, zstep):
        """The next free row (y, z, parity) of the current platform."""
        if self.y is None or self.z + zstep > BZ + SIZE - 2:
            self.y = self.take()
            self.z = BZ + 1
        z = self.z
        self.z += zstep + (1 if self.n % 2 == 0 else 0)
        self.n += 1
        return self.y, z, (self.n % 2)

    def platform(self, y):
        if y in self.made:
            return
        self.made.add(y)
        s = self.scene
        if y == LAYERS[0]:   # clear the natural terrain around the first platform
            for x in range(BX, BX + SIZE, 32):
                for z in range(BZ, BZ + SIZE, 32):
                    s.cmd(f'fill {x} {y} {z} {x + 31} {y + 11} {z + 31} minecraft:air 0')
        s.cmd(f'fill {BX} {y - 2} {BZ} {BX + SIZE - 1} {y - 1} {BZ + SIZE - 1} minecraft:stone 0')


class Packer:
    """Hands out cells in rows of the platforms: `cell()` -> (x, y, z), cells `stride` apart.
    Rows alternate their start so that both chunk edges (x % 16 == 0 and 15) are used."""

    def __init__(self, layers, stride=2, zstep=3):
        self.layers, self.stride, self.zstep = layers, stride, zstep
        self.y = self.x = self.z = None

    def newrow(self):
        self.y, self.z, parity = self.layers.row(self.zstep)
        self.x = BX + parity

    def cell(self, span=1):
        """One cell with room for `span` cells in x."""
        if self.x is None or self.x + self.stride * span > BX + SIZE - 1:
            self.newrow()
        x = self.x
        self.x += self.stride
        return x, self.y, self.z

    def gap(self, n=1):
        if self.x is not None:
            self.x += self.stride * n

    def take(self, n, span=1):
        return [self.cell(span) for _ in range(n)]


class Scene:
    def __init__(self):
        self.cmds = []       # the first phase
        self.late = []       # the last tick
        self.expect = {}     # (x, y, z) -> (block, data), what the scene placed
        self.tes = {}        # (x, y, z) -> block entity id expected
        self.layers = Layers(self)

    def cmd(self, c, late=False):
        (self.late if late else self.cmds).append(c)

    def block(self, x, y, z, name, data=0, tag=None, late=False, expect=True):
        if not block_ok(name):
            return
        if name in TWO_STEP and data:
            self.cmd(f'setblock {x} {y} {z} {full(name)} 0 replace', late)
        c = f'setblock {x} {y} {z} {full(name)} {data} replace' + (f' {tag}' if tag else '')
        self.cmd(c, late)
        if expect:
            self.expect[(x, y, z)] = (name, data)

    def fill(self, x1, y1, z1, x2, y2, z2, name, data=0, late=False):
        self.cmd(f'fill {x1} {y1} {z1} {x2} {y2} {z2} {full(name)} {data}', late)

    def summon(self, kind, x, y, z, nbt='', late=False):
        self.cmd(f'summon {kind} {x} {y} {z} {{{nbt}}}', late)


# ------------------------------------------------------------------ items
POTIONS = ['water', 'mundane', 'thick', 'awkward', 'night_vision', 'long_night_vision', 'invisibility', 'long_invisibility',
           'leaping', 'strong_leaping', 'long_leaping', 'fire_resistance', 'long_fire_resistance', 'swiftness',
           'strong_swiftness', 'long_swiftness', 'slowness', 'long_slowness', 'water_breathing', 'long_water_breathing',
           'healing', 'strong_healing', 'harming', 'strong_harming', 'poison', 'strong_poison', 'long_poison',
           'regeneration', 'strong_regeneration', 'long_regeneration', 'strength', 'strong_strength', 'long_strength',
           'weakness', 'long_weakness', 'luck']
ENCHANT_IDS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 16, 17, 18, 19, 20, 21, 22, 32, 33, 34, 35, 48, 49, 50, 51, 61, 62, 70, 71]
# 1.12 entity ids that a spawn egg or a spawner can carry
MOBS = ['bat', 'blaze', 'cave_spider', 'chicken', 'cow', 'creeper', 'donkey', 'elder_guardian', 'enderman', 'endermite',
        'evocation_illager', 'ghast', 'guardian', 'horse', 'husk', 'llama', 'magma_cube', 'mooshroom', 'mule', 'ocelot',
        'parrot', 'pig', 'polar_bear', 'rabbit', 'sheep', 'shulker', 'silverfish', 'skeleton', 'skeleton_horse', 'slime',
        'spider', 'squid', 'stray', 'vex', 'villager', 'vindication_illager', 'witch', 'wither_skeleton', 'wolf',
        'zombie', 'zombie_horse', 'zombie_pigman', 'zombie_villager', 'villager_golem', 'snowman', 'illusion_illager']
OLD_NAMES = {'cave_spider': 'CaveSpider', 'elder_guardian': 'ElderGuardian', 'evocation_illager': 'EvocationIllager',
             'magma_cube': 'LavaSlime', 'mooshroom': 'MushroomCow', 'ocelot': 'Ozelot', 'polar_bear': 'PolarBear',
             'skeleton_horse': 'SkeletonHorse', 'vindication_illager': 'VindicationIllager',
             'wither_skeleton': 'WitherSkeleton', 'zombie_horse': 'ZombieHorse', 'zombie_pigman': 'PigZombie',
             'zombie_villager': 'ZombieVillager', 'villager_golem': 'VillagerGolem', 'snowman': 'SnowMan',
             'illusion_illager': 'IllusionIllager'}
# Entity ids by era. 1.11 on: `minecraft:` snake_case names (as in `MOBS`); before: CamelCase ids and merged types.
CAMEL = {
    'zombie': 'Zombie', 'skeleton': 'Skeleton', 'creeper': 'Creeper', 'spider': 'Spider', 'cave_spider': 'CaveSpider',
    'silverfish': 'Silverfish', 'endermite': 'Endermite', 'blaze': 'Blaze', 'ghast': 'Ghast', 'witch': 'Witch', 'giant': 'Giant',
    'guardian': 'Guardian', 'shulker': 'Shulker', 'zombie_pigman': 'PigZombie', 'slime': 'Slime', 'magma_cube': 'LavaSlime',
    'enderman': 'Enderman', 'bat': 'Bat', 'squid': 'Squid', 'pig': 'Pig', 'cow': 'Cow', 'chicken': 'Chicken',
    'mooshroom': 'MushroomCow', 'sheep': 'Sheep', 'wolf': 'Wolf', 'ocelot': 'Ozelot', 'horse': 'EntityHorse',
    'rabbit': 'Rabbit', 'polar_bear': 'PolarBear', 'villager': 'Villager', 'villager_golem': 'VillagerGolem',
    'snowman': 'SnowMan', 'armor_stand': 'ArmorStand', 'painting': 'Painting', 'item_frame': 'ItemFrame',
    'leash_knot': 'LeashKnot', 'item': 'Item', 'xp_orb': 'XPOrb', 'minecart': 'MinecartRideable',
    'chest_minecart': 'MinecartChest', 'furnace_minecart': 'MinecartFurnace', 'hopper_minecart': 'MinecartHopper',
    'tnt_minecart': 'MinecartTNT', 'spawner_minecart': 'MinecartSpawner', 'commandblock_minecart': 'MinecartCommandBlock',
    'boat': 'Boat', 'tnt': 'PrimedTnt', 'falling_block': 'FallingSand', 'arrow': 'Arrow', 'spectral_arrow': 'SpectralArrow',
    'potion': 'ThrownPotion', 'egg': 'ThrownEgg', 'snowball': 'Snowball', 'ender_pearl': 'ThrownEnderpearl',
    'xp_bottle': 'ThrownExpBottle', 'eye_of_ender_signal': 'EyeOfEnderSignal', 'fireball': 'Fireball',
    'small_fireball': 'SmallFireball', 'dragon_fireball': 'DragonFireball', 'wither_skull': 'WitherSkull',
    'shulker_bullet': 'ShulkerBullet', 'ender_crystal': 'EnderCrystal', 'fireworks_rocket': 'FireworksRocketEntity',
    'area_effect_cloud': 'AreaEffectCloud',
}
# types that were one entity with a type field before 1.11: kind -> (CamelCase id, extra NBT)
MERGED = {
    'wither_skeleton': ('Skeleton', 'SkeletonType:1b'), 'stray': ('Skeleton', 'SkeletonType:2b'), 'husk': ('Zombie', 'ZombieType:6'),
    'zombie_villager': ('Zombie', 'ZombieType:2'), 'donkey': ('EntityHorse', 'Type:1'), 'mule': ('EntityHorse', 'Type:2'),
    'zombie_horse': ('EntityHorse', 'Type:3'), 'skeleton_horse': ('EntityHorse', 'Type:4'), 'elder_guardian': ('Guardian', 'Elder:1b'),
}
ABSENT = {   # kinds a version lacks
    # (thrown potions: vanilla's fix of key 705 crashes on them, so none before 1.11; 1.8 zombie villagers get a random profession)
    8: {'potion', 'zombie_villager', 'shulker', 'shulker_bullet', 'area_effect_cloud', 'spectral_arrow', 'dragon_fireball', 'polar_bear', 'husk', 'stray',
        'vex', 'evocation_illager', 'vindication_illager', 'illusion_illager', 'llama', 'llama_spit', 'evocation_fangs', 'parrot'},
    9: {'potion', 'polar_bear', 'husk', 'stray', 'vex', 'evocation_illager', 'vindication_illager', 'illusion_illager', 'llama',
        'llama_spit', 'evocation_fangs', 'parrot'},
    10: {'potion', 'vex', 'evocation_illager', 'vindication_illager', 'illusion_illager', 'llama', 'llama_spit', 'evocation_fangs', 'parrot'},
    11: {'parrot', 'illusion_illager'},
    12: set(),
}


def entity_form(kind, profession=None):
    """(summon id, extra NBT) of an entity kind in this era, None if the version lacks it."""
    if kind in ABSENT[ERA]:
        return None
    if ERA >= 11:
        return kind, ''
    if kind in MERGED:
        sid, extra = MERGED[kind]
        if kind == 'zombie_villager':
            extra = 'IsVillager:1b' if ERA == 8 else 'ZombieType:%d' % (1 + (profession or 0) % 5)
        return sid, extra
    return CAMEL[kind], ''


def eid(kind):
    """The quoted `id` for a kind inside NBT."""
    form = entity_form(kind)
    sid = form[0] if form else kind
    return q(sid if ERA <= 10 else 'minecraft:' + sid)


BANNER_PATTERNS = ['b', 'bs', 'ls', 'rs', 'ts', 'ms', 'drs', 'dls', 'ss', 'cr', 'sc', 'bt', 'tt', 'bts', 'tts', 'ld', 'rd',
                   'lud', 'rud', 'mc', 'mr', 'vh', 'hh', 'vhr', 'hhb', 'bo', 'cbo', 'br', 'tl', 'bl', 'tr', 'gra', 'gru',
                   'bri', 'sku', 'cre', 'flo', 'moj', 'cs']
RECORDS = ['record_13', 'record_cat', 'record_blocks', 'record_chirp', 'record_far', 'record_mall', 'record_mellohi',
           'record_stal', 'record_strad', 'record_ward', 'record_11', 'record_wait']
DAMAGEABLE = ['bow', 'carrot_on_a_stick', 'chainmail_boots', 'chainmail_chestplate', 'chainmail_helmet', 'chainmail_leggings',
              'diamond_axe', 'diamond_boots', 'diamond_chestplate', 'diamond_helmet', 'diamond_hoe', 'diamond_leggings',
              'diamond_pickaxe', 'diamond_shovel', 'diamond_sword', 'elytra', 'fishing_rod', 'flint_and_steel', 'golden_axe',
              'golden_boots', 'golden_chestplate', 'golden_helmet', 'golden_hoe', 'golden_leggings', 'golden_pickaxe',
              'golden_shovel', 'golden_sword', 'iron_axe', 'iron_boots', 'iron_chestplate', 'iron_helmet', 'iron_hoe',
              'iron_leggings', 'iron_pickaxe', 'iron_shovel', 'iron_sword', 'leather_boots', 'leather_chestplate',
              'leather_helmet', 'leather_leggings', 'shears', 'shield', 'stone_axe', 'stone_hoe', 'stone_pickaxe',
              'stone_shovel', 'stone_sword', 'wooden_axe', 'wooden_hoe', 'wooden_pickaxe', 'wooden_shovel', 'wooden_sword']


def sweep_stacks():
    """Every kind of item the flattening cares about, one stack each (no slot yet)."""
    out = []
    for n in ITEMS:
        for d in ITEM_VARIANTS.get(n, [0]):
            out.append(item(n, damage=d))
    # items whose damage is wear
    for i, n in enumerate(DAMAGEABLE):
        out.append(item(n, damage=(i * 7) % 60 + 1, tag='{Damage:%d,RepairCost:%d}' % ((i * 7) % 60 + 1, i % 5)))
    # damage values the flattening has no entry for, and non-damageable items with damage
    for n, d in [('stone', 7), ('stone', 15), ('wool', 16), ('wool', 100), ('log', 4), ('log', 15), ('planks', 6), ('dirt', 3),
                 ('sand', 2), ('apple', 3), ('stick', 9), ('dye', 16), ('leaves', 8), ('leaves2', 5), ('stonebrick', 7),
                 ('quartz_block', 5), ('prismarine', 3), ('anvil', 3), ('coal', 2), ('double_plant', 6), ('red_flower', 9),
                 ('tallgrass', 3), ('sapling', 6), ('cobblestone_wall', 2), ('monster_egg', 6), ('sponge', 2)]:
        out.append(item(n, damage=d))
    out.append(item('stone', damage=-1))
    # raw numeric ids (as written by a 1.7 server) are kept inside item tags
    for nid, d in [(1, 0), (35, 5), (5, 3), (17, 2), (351, 4), (397, 3), (383, 50), (373, 8261), (322, 1), (349, 3), (263, 1),
                   (160, 7), (159, 14), (251, 3), (425, 0), (397, 5), (298, 0), (276, 12)]:
        out.append(item('air', damage=d, numeric=nid))
    # potions in all four shapes
    if ERA == 8:       # before 1.9 a potion is its damage value: effect, level and extended / splash bits
        for d in list(range(0, 64)) + [8193, 8194, 8195, 8196, 8197, 8198, 8200, 8201, 8202, 8204, 8205, 8206, 8225, 8226, 8228, 8229, 8233,
                                         8234, 8236, 8257, 8258, 8259, 8260, 8261, 8262, 8264, 8265, 8266, 8270, 16385, 16386, 16387, 16388,
                                         16389, 16392, 16393, 16394, 16396, 16418, 16420, 16421, 16449, 16450, 16451, 16452, 16453, 16457, 16458]:
            out.append(item('potion', damage=d))
    for kind in ('potion', 'splash_potion', 'lingering_potion', 'tipped_arrow') if ERA >= 9 else ():
        for p in POTIONS:
            out.append(item(kind, count=1 if kind != 'tipped_arrow' else 7, tag='{Potion:%s}' % q('minecraft:' + p)))
        out.append(item(kind, tag='{CustomPotionEffects:[{Id:1b,Amplifier:1b,Duration:600,Ambient:0b,ShowParticles:1b},'
                                  '{Id:10b,Amplifier:0b,Duration:200}],CustomPotionColor:16711680,Potion:"minecraft:awkward"}'))
    out.append(item('potion', damage=8261))
    out.append(item('potion', damage=16453))
    out.append(item('potion', damage=32764))
    # enchantments, numeric ids
    for e in ENCHANT_IDS:
        out.append(item('enchanted_book', tag='{%s}' % stored_ench((e, 1 + e % 5), (34, 3))))
        out.append(item(DAMAGEABLE[e % len(DAMAGEABLE)], damage=e, tag='{%s,Damage:%d}' % (ench((e, 1 + e % 4), ((e + 7) % 35, 2)), e)))
    out.append(item('enchanted_book', tag='{StoredEnchantments:[{id:70s,lvl:1s},{id:71s,lvl:1s},{id:99s,lvl:1s}]}'))
    out.append(item('diamond_sword', tag='{ench:[{id:16s,lvl:32767s},{id:-1s,lvl:0s}],Unbreakable:1b,HideFlags:63}'))
    # spawn eggs, old style: the entity id in EntityTag
    for i, m in enumerate(MOBS):
        if entity_form(m) is None:
            continue
        out.append(item('spawn_egg', tag='{EntityTag:{id:%s}}' % eid(m)))
        if m in OLD_NAMES or i % 3 == 0:
            out.append(item('spawn_egg', tag='{EntityTag:{id:%s,CustomName:"Egg %d"}}' % (q(OLD_NAMES.get(m, m.title())), i)))
    out.append(item('spawn_egg', damage=50, tag='{EntityTag:{id:"minecraft:creeper",powered:1b}}'))
    out.append(item('spawn_egg'))
    for i in range(0, 130, 9):
        out.append(item('spawn_egg', damage=i))
    # names, lore, colors
    out.append(item('diamond_sword', tag='{%s,Damage:3}' % display('Blade', ['first line', 'second line'])))
    out.append(item('stick', tag='{%s}' % display('{"text":"json looking"}', ['{"text":"lore"}'])))
    out.append(item('paper', tag='{%s}' % display(r'§cColoured §lname', ['§7grey lore'])))
    out.append(item('stone', tag='{%s,ench:[],Unbreakable:0b}' % display('')))
    for i, c in enumerate([0, 255, 16711680, 65280, 8388608, 16777215, 10511680]):
        out.append(item(['leather_helmet', 'leather_chestplate', 'leather_leggings', 'leather_boots'][i % 4], damage=i,
                        tag='{%s}' % display(color=c)))
    # attribute modifiers
    out.append(item('iron_sword', tag='{AttributeModifiers:[{AttributeName:"generic.attackDamage",Name:"x",Amount:4.5d,Operation:0,'
                                      'UUIDMost:1L,UUIDLeast:2L,Slot:"mainhand"},{AttributeName:"generic.movementSpeed",Name:"y",'
                                      'Amount:0.1d,Operation:1,UUIDMost:3L,UUIDLeast:4L}]}'))
    # CanDestroy / CanPlaceOn
    out.append(item('diamond_pickaxe', tag='{CanDestroy:["minecraft:stone","dirt","minecraft:log","wool"]}'))
    out.append(item('stone', count=3, tag='{CanPlaceOn:["minecraft:grass","sand","minecraft:planks"]}'))
    out.append(item('diamond_pickaxe', tag='{CanDestroy:[1,2,3,35,17]}'))
    out.append(item('stone', tag='{CanPlaceOn:[2,12,5],CanDestroy:["minecraft:flowing_water"]}'))
    # banners and shields
    for i, c in enumerate(COLORS):
        pats = ','.join('{Pattern:%s,Color:%d}' % (q(BANNER_PATTERNS[(i * 3 + k) % len(BANNER_PATTERNS)]), (i + 5 * k) % 16)
                        for k in range(1 + i % 5))
        out.append(item('banner', damage=i, tag='{BlockEntityTag:{Base:%d,Patterns:[%s]}}' % (i, pats)))
        if i % 2 == 0:
            out.append(item('shield', damage=i, tag='{BlockEntityTag:{Base:%d,Patterns:[%s]}}' % (15 - i, pats)))
    out.append(item('banner', damage=4, tag='{BlockEntityTag:{Patterns:[{Pattern:"mr",Color:9}]},%s}' % display('Ominous Banner')))
    # skulls
    for t in range(6):
        out.append(item('skull', damage=t))
    out.append(item('skull', damage=3, tag='{SkullOwner:"Notch"}'))
    out.append(item('skull', damage=3, tag='{SkullOwner:{Id:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",Name:"Dinnerbone",'
                                             'Properties:{textures:[{Value:"eyJ0ZXh0dXJlcyI6e319",Signature:"sig"}]}}}'))
    # books
    out.append(item('written_book', tag='{title:"Title",author:"Me",generation:1,resolved:1b,pages:[%s,%s,"plain page"]}'
                    % (js({'text': 'page one', 'bold': True}), js({'text': '', 'extra': [{'text': 'two', 'clickEvent': {
                        'action': 'change_page', 'value': '1'}}, {'text': ' hover', 'hoverEvent': {
                            'action': 'show_text', 'value': {'text': 'hi'}}}]}))))
    out.append(item('written_book', tag='{title:"Old",author:"Ancient",generation:0,pages:["old \\"string\\" page"]}'))
    out.append(item('writable_book', tag='{pages:["draft one","draft two"]}'))
    # fireworks
    out.append(item('fireworks', count=3, tag='{Fireworks:{Flight:2b,Explosions:[{Type:1b,Colors:[I;255,16711680],FadeColors:[I;65280],'
                                              'Trail:1b,Flicker:0b},{Type:4b,Colors:[I;1],Trail:0b}]}}'))
    out.append(item('firework_charge', tag='{Explosion:{Type:3b,Colors:[I;65280],FadeColors:[I;1,2,3],Flicker:1b}}'))
    # nested containers in item tags
    inner = [item('stone', count=9, slot=0), item('golden_apple', damage=1, slot=3, tag='{%s}' % display('Gold')),
             item('dye', damage=4, slot=5), item('wool', damage=14, slot=26), item('air', numeric=35, damage=2, slot=1)]
    for i, c in enumerate(COLORS):
        out.append(item(c + '_shulker_box' if c != 'silver' else 'silver_shulker_box', tag='{BlockEntityTag:{Items:[%s],id:"minecraft:shulker_box"'
                        ',CustomName:%s,Lock:"key%d"}}' % (','.join(inner[:1 + i % 5]), q('Box ' + c), i)))
    out.append(item('chest', tag='{BlockEntityTag:{Items:[%s],id:"minecraft:chest"}}' % ','.join(inner)))
    out.append(item('chest', tag='{BlockEntityTag:{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:5L}}'))
    out.append(item('trapped_chest', tag='{BlockEntityTag:{Items:[%s]}}' % item('wool', damage=3, slot=0)))
    out.append(item('furnace', tag='{BlockEntityTag:{Items:[%s],BurnTime:100s,CookTime:50s,CookTimeTotal:200s}}' % item('iron_ore', slot=0)))
    out.append(item('mob_spawner', tag='{BlockEntityTag:{SpawnData:{id:"Zombie"},SpawnPotentials:[{Entity:{id:"Skeleton"},Weight:2}]}}'))
    out.append(item('beacon', tag='{BlockEntityTag:{Levels:3,Primary:1,Secondary:10}}'))
    out.append(item('command_block', tag='{BlockEntityTag:{Command:"say item",TrackOutput:0b}}'))
    out.append(item('noteblock', tag='{BlockEntityTag:{note:13b}}'))
    out.append(item('standing_banner', damage=3))
    out.append(item('flower_pot', count=2))
    out.append(item('monster_egg', damage=5))
    out.append(item('stained_glass_pane', damage=11, count=16))
    out.append(item('filled_map', damage=0, tag='{display:{MapColor:255}}'))
    out.append(item('filled_map', damage=1))
    out.append(item('filled_map', damage=2, tag='{map_is_scaling:1b}'))
    out.append(item('map', count=2))
    out.append(item('name_tag', tag='{%s}' % display('Named')))
    out.append(item('minecart', tag='{EntityTag:{id:"minecraft:minecart"}}'))
    out.append(item('armor_stand', tag='{EntityTag:{id:"minecraft:armor_stand",ShowArms:1b,Small:1b}}'))
    out.append(item('painting', tag='{EntityTag:{Motive:"Pigscene"}}'))
    out.append(item('item_frame', tag='{EntityTag:{Item:%s}}' % item('wool', damage=5)))
    out.append(item('knowledge_book', tag='{Recipes:["minecraft:stick","minecraft:wooden_slab","minecraft:stained_hardened_clay"]}'))
    out.append(item('totem_of_undying'))
    out.append(item('golden_apple', damage=1, count=3))
    out.append(item('banner', damage=1, tag='{display:{Name:"Plain"}}'))
    for r in RECORDS:
        out.append(item(r))
    return out


class Stacks:
    """A cursor over the sweep: containers take the next stacks, and wrap around at the end."""

    def __init__(self):
        self.stacks = sweep_stacks()
        if VALID_ITEMS is not None:      # the items this version lacks became sticks: keep one
            self.stacks = list(dict.fromkeys(self.stacks))
        self.i = 0

    def take(self, n):
        out = [self.stacks[(self.i + k) % len(self.stacks)] for k in range(n)]
        self.i += n
        return out

    def items(self, n, first=0):
        return slotted(self.take(n), first)


# ------------------------------------------------------------------ block entities
def sign_texts():
    """(a function: `js` depends on the era)"""
    return [
    [js({'text': 'Hello'}), js({'text': 'World', 'color': 'red', 'bold': True}), js({'text': ''}), js({'text': 'x', 'extra': [{'text': 'a', 'italic': True}]})],
    [js('plain string'), js({'text': 'click', 'clickEvent': {'action': 'run_command', 'value': '/say hi'}}),
     js({'text': 'hover', 'hoverEvent': {'action': 'show_text', 'value': {'text': 'tip'}}}), js({'text': 'selector', 'extra': [{'selector': '@p'}]})],
    [js({'translate': 'chat.type.text', 'with': ['a', 'b']}), js({'score': {'name': 'Upgrader', 'objective': 'kills'}}), js([{'text': 'l1'}, {'text': 'l2'}]), js('')],
    [q(''), q(''), q(''), q('')],
]


SKULL_PROFILE = ('Owner:{Id:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",Name:"Dinnerbone",Properties:{textures:[{Value:"eyJ0ZXh0dXJlcyI6e319"}]}}')
COMMANDS = ['say hi', 'tp @p ~ ~1 ~', 'execute @e[type=Zombie] ~ ~ ~ detect ~ ~-1 ~ stone 0 say hi', 'scoreboard players set @a kills 1',
            'give @p minecraft:wool 1 5', 'setblock ~ ~1 ~ minecraft:stone', 'testfor @a', 'playsound minecraft:entity.pig.ambient master @a']


def tag_for(name, data, k, stacks):
    """The tile entity data for one placed block, or None."""
    if name in ('chest', 'trapped_chest'):
        t = 'Items:' + stacks.items(27 if k % 4 else 20)
        if k % 5 == 0:
            t += ',CustomName:' + q('Chest %d' % k)
        if k % 7 == 3:
            t += ',Lock:"secret"'
        return t
    if name in ('furnace', 'lit_furnace'):
        t = 'Items:' + slotted(stacks.take(3)) + (',BurnTime:%ds,CookTime:%ds,CookTimeTotal:200s' % ((32000 if name == 'lit_furnace' else 0), 10 + k))
        return t + (',CustomName:"Burner"' if k % 2 else '') + (',RecipesUsed:{"minecraft:iron_ingot":2}' if k % 3 == 0 else '')
    if name in ('dispenser', 'dropper'):
        return 'Items:' + stacks.items(9) + (',CustomName:%s' % q(name + ' ' + str(k)) if k % 2 else '')
    if name == 'hopper':
        return 'Items:' + stacks.items(5) + ',TransferCooldown:%d' % (k % 8) + (',CustomName:"Funnel"' if k % 2 else '')
    if name == 'brewing_stand':      # the block data follows the bottles in the first three slots
        picks = stacks.take(3)
        slots = ['{Slot:%db,%s' % (i, picks[i][1:]) for i in range(3) if data >> i & 1]
        slots += ['{Slot:3b,%s' % stacks.take(1)[0][1:], '{Slot:4b,%s' % item('blaze_powder', 3)[1:]]
        return ('Items:[' + ','.join(slots) + ']' + ',BrewTime:%ds,Fuel:%db' % (k * 3, 20 - k)
                + (',CustomName:"Brewery"' if k % 2 else ''))
    if name == 'enchanting_table':
        return 'CustomName:' + q('Enchant ' + str(k))
    if name == 'beacon':
        return 'Levels:%d,Primary:%d,Secondary:%d' % (k % 5, 1 + k % 10, k % 3 and 1 + k % 10)
    if name == 'jukebox':
        return ('RecordItem:' + item(RECORDS[k % len(RECORDS)])) if data else 'Record:%d' % (2256 + k % 12)
    if name == 'noteblock':
        return 'note:%db,powered:%db' % (k % 25, k % 2)
    if name in ('command_block', 'repeating_command_block', 'chain_command_block'):
        t = 'Command:%s,SuccessCount:%d,TrackOutput:%db,powered:0b,auto:0b,conditionMet:%db' % (
            q(COMMANDS[k % len(COMMANDS)]), k % 4, k % 2, data // 8)
        t += ',LastOutput:%s' % js({'text': '[@] done', 'color': 'white'}) if k % 2 else ''
        t += ',CustomName:%s' % q('Cmd ' + str(k)) if k % 3 == 0 else ''
        return t + ',UpdateLastExecution:1b,LastExecution:%dL' % (1000 + k)
    if name == 'structure_block':
        return ('name:"scene:test%d",author:"Me",metadata:"meta",posX:%d,posY:%d,posZ:%d,sizeX:%d,sizeY:%d,sizeZ:%d,rotation:%s,mirror:%s,'
                'mode:%s,integrity:%d.5f,seed:%dL,ignoreEntities:%db,powered:0b,showair:%db,showboundingbox:1b') % (
                    k, k % 3 - 1, 1 + k % 2, -k % 3, 2 + k % 5, 3, 4, q(['NONE', 'CLOCKWISE_90', 'CLOCKWISE_180', 'COUNTERCLOCKWISE_90'][k % 4]),
                    q(['NONE', 'LEFT_RIGHT', 'FRONT_BACK'][k % 3]), q(['SAVE', 'LOAD', 'CORNER', 'DATA'][data & 3]), k % 2, k * 77, k % 2, k % 2)
    if name in ('standing_sign', 'wall_sign'):
        texts = sign_texts()
        text = texts[k % len(texts)]
        return ','.join('Text%d:%s' % (i + 1, text[i]) for i in range(4))
    if name == 'mob_spawner':
        mob = [m for m in MOBS if entity_form(m)][k % len([m for m in MOBS if entity_form(m)])]
        spawn = 'SpawnData:{id:%s}' % (q(OLD_NAMES.get(mob, mob.title())) if k % 2 else eid(mob))
        if k % 3 == 0:
            spawn = ('SpawnData:{id:"minecraft:zombie",HandItems:[%s,{}],ArmorItems:[{},{},{},%s],CustomName:"Spawned",Health:5.0f},'
                     'SpawnPotentials:[{Entity:{id:"minecraft:skeleton",HandItems:[%s,{}]},Weight:2},{Entity:{id:"Pig"},Weight:1}]'
                     % (item('stone_sword', damage=3), item('log', damage=2), item('air', numeric=261)))
        return (spawn + ',MaxNearbyEntities:%ds,RequiredPlayerRange:1s,SpawnCount:%ds,SpawnRange:%ds,Delay:%ds,MinSpawnDelay:%ds,MaxSpawnDelay:%ds'
                % (6 + k % 3, 3 + k % 4, 4 + k % 2, 20 + k, 200, 800))
    if name.endswith('shulker_box'):
        return 'Items:' + stacks.items(27 if k % 2 else 12) + (',CustomName:%s' % q('Shulk ' + str(k)) if k % 3 == 0 else '') + (
            ',Lock:"k"' if k % 5 == 1 else '')
    if name == 'end_gateway':
        return 'Age:%dL,ExactTeleport:%db,ExitPortal:{X:%d,Y:%d,Z:%d}' % (k * 10, k % 2, 10 + k, 70, 20 - k)
    if name in ('unpowered_comparator', 'powered_comparator'):
        return 'OutputSignal:%d' % (k % 16)
    if name == 'skull':
        return None   # see skull()
    return None


# ------------------------------------------------------------------ the block sweep
# families the sweep does not place with the generic rule
SPECIAL = set(['air', 'bed', 'wooden_door', 'iron_door', 'spruce_door', 'birch_door', 'jungle_door', 'acacia_door', 'dark_oak_door',
               'double_plant', 'standing_banner', 'wall_banner', 'skull', 'flower_pot', 'piston_head', 'piston_extension',
               'daylight_detector', 'daylight_detector_inverted', 'observer', 'water', 'flowing_water', 'lava', 'flowing_lava',
               'fire', 'portal', 'frosted_ice', 'lit_redstone_lamp', 'hopper'])
# the game recomputes these when they are added: place the block with data 0 first, then the real state (the second
# placement replaces a block of the same kind and does not run `onBlockAdded`)
TWO_STEP = set(['rail', 'golden_rail', 'detector_rail', 'activator_rail', 'redstone_wire', 'unpowered_repeater', 'powered_repeater',
                'unpowered_comparator', 'powered_comparator', 'dispenser', 'dropper', 'piston', 'sticky_piston', 'tripwire_hook',
                'tripwire', 'observer', 'daylight_detector', 'daylight_detector_inverted'])
# placed in the last tick: their state is rewritten by a scheduled tick
LATE_NAMES = set(['powered_repeater', 'powered_comparator'])
# soil under plants: name -> (block, data)
FLOORS = {
    'sapling': ('grass', 0), 'tallgrass': ('grass', 0), 'red_flower': ('grass', 0), 'yellow_flower': ('grass', 0),
    'wheat': ('farmland', 7), 'carrots': ('farmland', 7), 'potatoes': ('farmland', 7), 'beetroots': ('farmland', 7),
    'pumpkin_stem': ('farmland', 7), 'melon_stem': ('farmland', 7), 'nether_wart': ('soul_sand', 0), 'cactus': ('sand', 0),
    'deadbush': ('sand', 1), 'brown_mushroom': ('dirt', 2), 'red_mushroom': ('dirt', 2), 'waterlily': ('water', 0),
    'reeds': ('sand', 0), 'chorus_flower': ('end_stone', 0), 'chorus_plant': ('end_stone', 0),
}
# blocks that need something solid at their side or above: surrounded by the supports
POCKETS = {'torch': 'stone', 'redstone_torch': 'stone', 'lever': 'stone', 'stone_button': 'stone', 'wooden_button': 'stone',
           'ladder': 'stone', 'wall_sign': 'stone', 'wall_banner': 'stone', 'vine': 'stone', 'tripwire_hook': 'stone',
           'cocoa': 'log'}


def pocket(scene, x, y, z, support):
    block, data = ('stone', 0) if support == 'stone' else ('log', 3)
    for dx, dy, dz in ((1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0)):
        scene.block(x + dx, y + dy, z + dz, block, data, expect=False)


def sweep_blocks(scene, stacks):
    plain = Packer(scene.layers, stride=2, zstep=3)
    pockets = Packer(scene.layers, stride=3, zstep=4)
    k = 0
    for name in sorted(BLOCK_STATES, key=lambda n: BLOCK_IDS[n]):
        if name in SPECIAL or not block_ok(name):
            continue
        datas = BLOCK_STATES[name]
        pk = name in POCKETS
        packer = pockets if pk else plain
        cells = packer.take(len(datas))
        packer.gap()
        for (x, y, z), d in zip(cells, datas):
            k += 1
            if name in FLOORS:
                fb, fd = FLOORS[name]
                scene.block(x, y - 1, z, fb, fd, expect=False)
                if name == 'reeds':
                    scene.block(x + 1, y - 1, z, 'water', 0, expect=False)
            if pk:
                pocket(scene, x, y, z, POCKETS[name])
            t = tag_for(name, d, k, stacks)
            scene.block(x, y, z, name, d, '{' + t + '}' if t else None, late=name in LATE_NAMES)


# ------------------------------------------------------------------ the player: a bot that answers the server
def pos_long(x, y, z):
    return struct.pack('>Q', ((x & 0x3FFFFFF) << 38) | ((y & 0xFFF) << 26) | (z & 0x3FFFFFF))


def make_player(Bot, varint, parse_varint, mcstring):
    """A Bot (protocol 340) that keeps the connection alive, confirms teleports and can act: dig, place, use, attack."""

    class Player(Bot):
        def __init__(self, *args):
            self.entities = {}    # entity id -> mob type id
            self.pos = (0.0, 0.0, 0.0)
            self.lock = threading.Lock()
            super().__init__(*args)

        def idle(self):
            self.sock.settimeout(1.0)
            while not self.stop.is_set():
                try:
                    pid, data = self.receive()
                except socket.timeout:
                    continue
                except (EOFError, OSError):
                    break
                try:
                    self.handle(pid, data)
                except Exception:
                    pass

        def handle(self, pid, data):
            if self.protocol < 340:     # 1.8.9 (47) to 1.11.2 (316): only stay joined
                if self.protocol == 47 and pid == 0x00:       # keep alive: a VarInt
                    self.send(0x00, data)
                elif self.protocol > 47 and pid == 0x1F:      # keep alive: a long
                    self.send(0x0B, data[:8])
                elif self.protocol > 47 and pid == 0x2E:      # player position and look: confirm the teleport
                    self.send(0x00, varint(parse_varint(data, 33)[1]))
                return
            if pid == 0x1F:    # keep alive
                self.send(0x0B, data[:8])
            elif pid == 0x2F:  # player position and look
                x, y, z = struct.unpack('>ddd', data[:24])
                self.pos = (x, y, z)
                tid = parse_varint(data, 33)[1]
                self.send(0x00, varint(tid))
                self.send(0x0E, struct.pack('>dddffB', x, y, z, 0.0, 0.0, 1))
            elif pid == 0x03:  # spawn mob
                p, eid = parse_varint(data, 0)
                self.entities[eid] = data[p + 16]   # skip the uuid, the type byte

        def use_map_18(self, slot):
            """1.8 only: select a hotbar slot and right click in the air (an empty map there becomes a map)."""
            self.send(0x09, struct.pack('>h', slot))
            time.sleep(0.3)
            self.send(0x08, struct.pack('>qBhBBB', -1, 255, -1, 0, 0, 0))
            time.sleep(1.0)

        def move(self, x, y, z):
            self.pos = (x, y, z)
            self.send(0x0D, struct.pack('>dddB', x, y, z, 1))

        def held(self, slot):
            self.send(0x1A, struct.pack('>h', slot))

        def use_item(self, hand=0):
            self.send(0x20, varint(hand))

        def dig(self, x, y, z, face=1):
            self.send(0x14, varint(0) + pos_long(x, y, z) + bytes([face]))
            self.send(0x14, varint(2) + pos_long(x, y, z) + bytes([face]))

        def place(self, x, y, z, face=1):
            self.send(0x1F, pos_long(x, y, z) + varint(face) + varint(0) + struct.pack('>fff', 0.5, 1.0, 0.5))

        def attack(self, eid):
            self.send(0x0A, varint(eid) + varint(1))

        def swing(self):
            self.send(0x1D, varint(0))

        def chat(self, text):
            self.send(0x02, mcstring(text))

        def respawn(self):
            self.send(0x03, varint(0))

    return Player


# ------------------------------------------------------------------ multi-block and tile entity families
DOORS = ['wooden_door', 'spruce_door', 'birch_door', 'jungle_door', 'acacia_door', 'dark_oak_door', 'iron_door']
FACE_XZ = {0: (0, 1), 1: (-1, 0), 2: (0, -1), 3: (1, 0)}                      # horizontal index -> (dx, dz)
FACE_XYZ = {0: (0, -1, 0), 1: (0, 1, 0), 2: (0, 0, -1), 3: (0, 0, 1), 4: (-1, 0, 0), 5: (1, 0, 0)}   # piston style facing


def doors(scene):
    pk = Packer(scene.layers, stride=2, zstep=4)
    for name in DOORS:
        for lower in range(8):
            for upper in (8, 9, 10, 11):
                x, y, z = pk.cell()
                scene.block(x, y, z, name, lower)
                scene.block(x, y + 1, z, name, upper)
        for lower in (0, 5):           # a door without its upper half
            x, y, z = pk.cell()
            scene.block(x, y, z, name, lower)
        x, y, z = pk.cell()            # and an upper half alone
        scene.block(x, y + 1, z, name, 9)
        pk.gap()


def double_plants(scene):
    pk = Packer(scene.layers, stride=2, zstep=3)
    for lower in range(6):
        x, y, z = pk.cell()
        scene.block(x, y - 1, z, 'grass', 0, expect=False)
        scene.block(x, y, z, 'double_plant', lower)
        scene.block(x, y + 1, z, 'double_plant', 8, expect=False)
    x, y, z = pk.cell()
    scene.block(x, y - 1, z, 'grass', 0, expect=False)
    scene.block(x, y, z, 'double_plant', 4)    # without upper half
    x, y, z = pk.cell()
    scene.block(x, y + 1, z, 'double_plant', 8, expect=False)


def beds(scene):
    pk = Packer(scene.layers, stride=4, zstep=4)
    k = 0
    for color in range(16):
        for f in range(4):
            x, y, z = pk.cell()
            dx, dz = FACE_XZ[f]
            occupied = 4 if (color + f) % 5 == 0 else 0
            scene.block(x, y, z, 'bed', f, '{color:%d}' % color)
            scene.block(x + dx, y, z + dz, 'bed', 8 + f + occupied, '{color:%d}' % color)
            k += 1
    for f in range(4):          # a foot or a head alone
        x, y, z = pk.cell()
        scene.block(x, y, z, 'bed', f, '{color:%d}' % (f * 3))
        x, y, z = pk.cell()
        scene.block(x, y, z, 'bed', 8 + f, '{color:%d}' % (f * 4 + 1))


def banner_tag(color, k):
    n = k % 5
    pats = ','.join('{Pattern:%s,Color:%d}' % (q(BANNER_PATTERNS[(color * 5 + k + j) % len(BANNER_PATTERNS)]), (color + 3 * j + k) % 16)
                    for j in range(n))
    t = 'Base:%d' % color
    if pats:
        t += ',Patterns:[%s]' % pats
    if k % 7 == 0:
        t += ',CustomName:%s' % q('Banner %d' % k)
    return '{%s}' % t


def banners(scene):
    pk = Packer(scene.layers, stride=2, zstep=3)
    pp = Packer(scene.layers, stride=3, zstep=4)
    k = 0
    for color in range(16):
        for rot in range(16):
            x, y, z = pk.cell()
            scene.block(x, y, z, 'standing_banner', rot, banner_tag(color, k))
            k += 1
        for f in (2, 3, 4, 5):
            x, y, z = pp.cell()
            pocket(scene, x, y, z, 'stone')
            scene.block(x, y, z, 'wall_banner', f, banner_tag(color, k))
            k += 1


def skulls(scene):
    pk = Packer(scene.layers, stride=2, zstep=3)
    k = 0
    for data in BLOCK_STATES['skull']:
        for kind in range(6 if ERA >= 9 else 5):
            for rot in (range(16) if data == 1 else [0] if data in (0, 8) else [k % 16]):
                x, y, z = pk.cell()
                tag = 'SkullType:%db,Rot:%db' % (kind, rot)
                if kind == 3:
                    tag += ',' + SKULL_PROFILE if (k + rot) % 2 == 0 else ''
                scene.block(x, y, z, 'skull', data, '{%s}' % tag)
                k += 1


POT_CONTENTS = ([('', 0)] + [('red_flower', d) for d in range(9)] + [('yellow_flower', 0)] + [('sapling', d) for d in range(6)] +
                [('red_mushroom', 0), ('brown_mushroom', 0), ('deadbush', 0), ('tallgrass', 2), ('cactus', 0), ('tallgrass', 0)])


def flower_pots(scene):
    pk = Packer(scene.layers, stride=2, zstep=3)
    for i in range(len(POT_CONTENTS) * 2):
        content, data = POT_CONTENTS[i % len(POT_CONTENTS)]
        x, y, z = pk.cell()
        if i >= len(POT_CONTENTS) and content:   # the second round names the item by its numeric id
            it = str(BLOCK_IDS[content])
        else:
            it = q('minecraft:' + content if content else '')
        scene.block(x, y, z, 'flower_pot', (i * 5) % 16 if i % 3 == 0 else 0, '{Item:%s,Data:%d}' % (it, data), expect=False)


def lamps_and_lit_things(scene):
    pk = Packer(scene.layers, stride=3, zstep=3)
    x, y, z = pk.cell()
    scene.block(x + 1, y, z, 'redstone_block', 0)
    scene.block(x, y, z, 'lit_redstone_lamp', 0)
    x, y, z = pk.cell()
    scene.block(x, y, z, 'redstone_lamp', 0)
    x, y, z = pk.cell()
    scene.block(x, y - 1, z, 'grass', 0, expect=False)
    scene.block(x, y, z, 'pumpkin', 0)
    x, y, z = pk.cell()      # carved pumpkins on grass, dirt and podzol, snow on top (the flattening treats them apart)
    for floor, fdata in (('grass', 0), ('dirt', 0), ('dirt', 2), ('mycelium', 0), ('grass', 0), ('dirt', 1)):
        scene.block(x, y - 1, z, floor, fdata, expect=False)
        scene.block(x, y, z, 'lit_pumpkin' if fdata else 'pumpkin', x % 4)
        x, y, z = pk.cell()
    for floor, fdata in (('grass', 0), ('dirt', 0), ('dirt', 2), ('mycelium', 0), ('grass', 0)):
        scene.block(x, y - 1, z, floor, fdata, expect=False)
        scene.block(x, y, z, 'snow_layer', 1 + x % 7)
        x, y, z = pk.cell()
    # snowy grass (a snow layer on top)
    for i in range(4):
        scene.block(x, y - 1, z, ['grass', 'dirt', 'mycelium', 'dirt'][i], [0, 2, 0, 0][i], expect=False)
        scene.block(x, y, z, 'snow', 0)
        x, y, z = pk.cell()


def notes_and_spawners(scene):
    pk = Packer(scene.layers, stride=2, zstep=3)
    for note in range(25):
        for powered in (0, 1):
            x, y, z = pk.cell()
            scene.block(x, y, z, 'noteblock', 0, '{note:%db,powered:%db}' % (note, powered))
    pk.gap()
    for i in range(12):
        x, y, z = pk.cell()
        scene.block(x, y, z, 'mob_spawner', 0, '{%s}' % tag_for('mob_spawner', 0, i, None))
    for i, rec in enumerate(RECORDS):
        x, y, z = pk.cell()
        scene.block(x, y, z, 'jukebox', 1, '{RecordItem:%s}' % item(rec))
        x, y, z = pk.cell()
        scene.block(x, y, z, 'jukebox', 0, '{Record:%d}' % (2256 + i))


def edges(scene):
    """The blocks that UpgradeData has to revisit, on both sides of chunk borders, in all four corners and edges."""
    virtual = ['chest', 'trapped_chest', 'noteblock', 'bed', 'fire', 'oak_stairs', 'stone_stairs', 'brick_stairs', 'stone_brick_stairs',
               'nether_brick_stairs', 'sandstone_stairs', 'spruce_stairs', 'birch_stairs', 'jungle_stairs', 'quartz_stairs',
               'acacia_stairs', 'dark_oak_stairs', 'red_sandstone_stairs', 'purpur_stairs', 'redstone_wire', 'fence',
               'nether_brick_fence', 'spruce_fence', 'birch_fence', 'jungle_fence', 'dark_oak_fence', 'acacia_fence',
               'unpowered_repeater', 'powered_repeater', 'iron_bars', 'glass_pane', 'stained_glass_pane', 'fence_gate',
               'spruce_fence_gate', 'birch_fence_gate', 'jungle_fence_gate', 'dark_oak_fence_gate', 'acacia_fence_gate',
               'tripwire', 'cobblestone_wall', 'chorus_plant', 'melon_stem', 'pumpkin_stem', 'snow_layer', 'cauldron',
               'standing_sign', 'mob_spawner']
    virtual = [n for n in virtual if block_ok(n)]
    y = scene.layers.take()
    k = 0
    cells = []
    for chunk in range(1, 6):            # borders x = 16 * (9 + chunk): x - 1 and x are on different chunks
        bx = BX + 16 * chunk
        for i in range(0, SIZE, 2):
            cells.append((bx - 1, BZ + 1 + i))          # last column of a chunk, z odd
            cells.append((bx, BZ + 2 + i))              # first column of the next chunk, z even
            cells.append((BX + 1 + i, bz_of(chunk) - 1))
            cells.append((BX + 2 + i, bz_of(chunk)))
    seen = set()
    for n, name in enumerate(virtual):
        datas = BLOCK_STATES[name][:3]
        for j, d in enumerate(datas):
            for corner in range(8):
                pos = cells[(n * 24 + j * 8 + corner * 31) % len(cells)]
                while pos in seen or pos[1] >= BZ + SIZE - 1:
                    pos = (pos[0], (pos[1] + 2) if pos[1] + 2 < BZ + SIZE - 1 else BZ + 1 + (pos[1] % 2))
                    if pos in seen:
                        pos = (pos[0] + 2, pos[1])
                seen.add(pos)
                x, z = pos
                if name in FLOORS:
                    scene.block(x, y - 1, z, FLOORS[name][0], FLOORS[name][1], expect=False)
                t = tag_for(name, d, k, None) if name in ('standing_sign', 'wall_sign', 'mob_spawner') else None
                if name in ('chest', 'trapped_chest'):
                    t = 'Items:[%s]' % item('wool', damage=k % 16, slot=0)
                k += 1
                if name == 'bed':
                    scene.block(x, y, z, 'bed', d % 4, '{color:%d}' % (k % 16))
                    continue
                scene.block(x, y, z, name, d, '{%s}' % t if t else None, late=name in LATE_NAMES)


def bz_of(chunk):
    return BZ + 16 * chunk


def transient_blocks(scene, stacks):
    """States the game rewrites within a tick or two: placed in the last tick, before the save."""
    pk = Packer(scene.layers, stride=4, zstep=4)
    late = True
    # extended pistons, with their heads, powered from behind
    for sticky in (0, 1):
        for f in range(6):
            x, y, z = pk.cell()
            if f in (0, 1):
                y += 3
                scene.block(x, y - 1, z, 'stone', 0, late=late, expect=False)
            dx, dy, dz = FACE_XYZ[f]
            name = 'sticky_piston' if sticky else 'piston'
            scene.block(x - dx, y - dy, z - dz, 'redstone_block', 0, late=late, expect=False)
            scene.block(x, y, z, name, 8 + f, late=late)
            scene.block(x + dx, y + dy, z + dz, 'piston_head', f + 8 * sticky, late=late)
    # retracted pistons of every facing
    for f in range(6):
        for name in ('piston', 'sticky_piston'):
            x, y, z = pk.cell()
            scene.block(x, y, z, name, f, late=late)
    # moving pistons
    for i in range(12):
        x, y, z = pk.cell()
        f = i % 6
        bid, bdata = [(1, 0), (35, 5), (54, 2), (17, 1), (5, 3), (159, 9), (95, 14), (44, 11), (155, 2), (24, 1), (3, 2), (12, 1)][i]
        scene.block(x, y, z, 'piston_extension', [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13][i],
                    '{blockId:%d,blockData:%d,facing:%d,progress:%s,extending:%db,source:%db}' % (
                        bid, bdata, f, ['0.0f', '0.5f', '1.0f'][i % 3], i % 2, (i // 2) % 2), late=late)
    # daylight detectors
    for name in ('daylight_detector', 'daylight_detector_inverted'):
        for power in range(16):
            x, y, z = pk.cell()
            scene.block(x, y, z, name, power, late=late)
    # hoppers, the disabled ones next to a power source
    for data in BLOCK_STATES['hopper']:
        x, y, z = pk.cell()
        if data >= 8:
            scene.block(x + 1, y, z, 'redstone_block', 0, late=late, expect=False)
        scene.block(x, y, z, 'hopper', data, '{Items:%s,TransferCooldown:%d}' % (stacks.items(5), data % 4), late=late)
    # observers, powered ones included
    for data in range(6) if block_ok('observer') else ():   # (the powered states only come from a pulse that takes ticks)
        x, y, z = pk.cell()
        scene.block(x, y, z, 'observer', data, late=late)
    # liquids of every level, still and flowing
    for name in ('water', 'flowing_water', 'lava', 'flowing_lava'):
        for level in range(16):
            x, y, z = pk.cell()
            scene.block(x, y, z, name, level, late=late)
    # fire of every age, frosted ice
    for age in range(16):
        x, y, z = pk.cell()
        scene.block(x, y, z, 'fire', age, late=late)
    for age in range(4) if block_ok('frosted_ice') else ():
        x, y, z = pk.cell()
        scene.block(x, y, z, 'frosted_ice', age, late=late)
    # nether portals in both axes: obsidian frames, then the portal blocks with one fill
    for axis in (1, 2):
        x, y, z = pk.cell(span=2)
        pk.gap()
        if axis == 1:
            scene.fill(x, y, z, x + 3, y + 4, z, 'obsidian', 0, late=late)
            scene.fill(x + 1, y + 1, z, x + 2, y + 3, z, 'portal', 1, late=late)
            scene.expect[(x + 1, y + 1, z)] = ('portal', 1)
        else:
            scene.fill(x, y, z, x, y + 4, z + 3, 'obsidian', 0, late=late)
            scene.fill(x, y + 1, z + 1, x, y + 3, z + 2, 'portal', 2, late=late)
            scene.expect[(x, y + 1, z + 1)] = ('portal', 2)


def top(scene, stacks):
    """The highest sections: a platform at y = 244 and columns of blocks up to the build limit."""
    scene.layers.platform(244)
    for i in range(16):
        for y in range(240, 256):
            scene.block(BX + 2 + 2 * i, y, BZ + 2, 'wool' if y % 2 else 'stained_hardened_clay', (i + y) % 16)
    x, z = BX + 2, BZ + 6
    scene.block(x, 255, z, 'chest', 2, '{Items:%s,CustomName:"Top chest"}' % stacks.items(27))
    scene.block(x + 2, 255, z, 'standing_sign', 7, '{Text1:%s,Text2:%s}' % (js({'text': 'y=255'}), js({'text': 'limit'})))
    scene.block(x + 4, 255, z, 'command_block', 1, '{Command:"say top",TrackOutput:0b}')
    scene.block(x + 6, 254, z, 'skull', 1, '{SkullType:5b,Rot:4b}')
    scene.block(x + 8, 253, z, 'bed', 1, '{color:14}')
    scene.block(x + 7, 253, z, 'bed', 9, '{color:14}')
    scene.block(x + 10, 255, z, 'standing_banner', 3, banner_tag(5, 3))
    scene.block(x + 12, 250, z, 'shulker_box' if False else 'purple_shulker_box', 5, '{Items:%s}' % stacks.items(5))
    scene.block(x + 14, 255, z, 'mob_spawner', 0, '{SpawnData:{id:%s},MaxNearbyEntities:6s,RequiredPlayerRange:1s}' % eid('ghast'))
    for i in range(8):      # and the very bottom of the world, next to the bedrock
        scene.block(BX + 2 + 2 * i, 5 + i, BZ + 3, 'sponge', i % 2)
        scene.block(BX + 2 + 2 * i, 1, BZ + 5, 'wool', i)


def build_blocks(scene, stacks):
    sweep_blocks(scene, stacks)
    doors(scene)
    double_plants(scene)
    beds(scene)
    banners(scene)
    skulls(scene)
    flower_pots(scene)
    lamps_and_lit_things(scene)
    notes_and_spawners(scene)
    edges(scene)
    top(scene, stacks)


# ------------------------------------------------------------------ entities
def uuid_tag(n):
    return 'UUIDMost:%dL,UUIDLeast:%dL' % (UUID_HI, n)


STAY = 'NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b'


def equipment(i, stacks=None):
    """Hand and armor items with numeric damage and enchantments, as 1.12 mobs carry them."""
    weapons = ['diamond_sword', 'bow', 'iron_axe', 'golden_pickaxe', 'shield', 'fishing_rod', 'stone_hoe', 'wooden_shovel']
    hand = item(weapons[i % 8], damage=i % 40, tag='{%s}' % ench(((i * 3) % 22, 1 + i % 3), (34, 2)) if i % 2 else None)
    off = item('shield', damage=i % 5, tag='{BlockEntityTag:{Base:%d,Patterns:[{Pattern:"bs",Color:%d}]}}' % (i % 16, (i + 4) % 16)) if i % 3 == 0 else '{}'
    armor = [item('leather_boots', damage=i % 20, tag='{%s}' % display(color=(i * 99991) % 16777216)),
             item(['iron', 'golden', 'diamond', 'chainmail'][i % 4] + '_leggings', damage=i % 30),
             item(['iron', 'golden', 'diamond', 'chainmail'][(i + 1) % 4] + '_chestplate', damage=i % 30, tag='{%s}' % ench((0, 2), (7, 3))),
             (item('skull', damage=i % 6) if i % 4 == 0 else item('pumpkin') if i % 4 == 1 else item('wool', damage=i % 16) if i % 4 == 2 else item('diamond_helmet', damage=7, tag='{%s}' % ench((5, 2))))]
    if ERA == 8:      # before 1.9: one list, the held item first and then boots, leggings, chestplate and helmet
        return 'Equipment:[%s,%s],DropChances:[0.085f,0.0f,1.0f,%.2ff,%.3ff]' % (hand, ','.join(armor), 0.1 * (i % 5), 0.05 * (i % 7))
    return ('HandItems:[%s,%s],ArmorItems:[%s],HandDropChances:[0.085f,%.3ff],ArmorDropChances:[0.085f,0.0f,1.0f,%.2ff]'
            % (hand, off, ','.join(armor), 0.05 * (i % 7), 0.1 * (i % 5)))


EFFECTS = 'ActiveEffects:[{Id:1b,Amplifier:1b,Duration:600,Ambient:0b,ShowParticles:1b},{Id:10b,Amplifier:0b,Duration:99999,Ambient:1b,ShowParticles:0b}]'
ATTRS = ('Attributes:[{Name:"generic.maxHealth",Base:30.0d,Modifiers:[{Name:"bonus",Amount:2.0d,Operation:0,UUIDMost:7L,UUIDLeast:8L}]},'
         '{Name:"generic.movementSpeed",Base:0.2d},{Name:"generic.followRange",Base:40.0d}]')
TRADE_ITEMS = [('wheat', 0, 18), ('wool', 5, 1), ('dye', 4, 3), ('stained_hardened_clay', 11, 2), ('paper', 0, 24), ('coal', 1, 15),
               ('fish', 3, 6), ('cooked_fish', 1, 6), ('stone', 5, 8), ('log', 2, 4), ('record_13', 0, 1), ('golden_apple', 1, 1)]


def trades(n):
    out = []
    for i in range(n):
        b = TRADE_ITEMS[i % len(TRADE_ITEMS)]
        s = TRADE_ITEMS[(i * 5 + 3) % len(TRADE_ITEMS)]
        t = '{buy:%s,sell:%s,maxUses:%d,uses:%d,rewardExp:%db' % (item(b[0], b[2], b[1]), item(s[0], 1, s[1]), 7 + i, i % 3, i % 2)
        if i % 3 == 1:
            t += ',buyB:%s' % item('emerald', 1)
        if i % 4 == 2:
            t = t.replace('sell:' + item(s[0], 1, s[1]), 'sell:' + item('enchanted_book', 1, 0, '{%s}' % stored_ench((34 + i % 2, 2), (70, 1))))
        out.append(t + '}')
    return 'Offers:{Recipes:[%s]}' % ','.join(out)


def entity_list(stacks):
    """(summon id, nbt, late). Fixed uuids are added by the caller."""
    E = []

    def add(kind, nbt='', late=False, stay=True):
        E.append((kind, nbt, late, stay))

    def ride(mount, mount_nbt, riders):
        """From 1.9 a mount lists its `Passengers`; before, the rider has a `Riding` mount (one rider only)."""
        if ERA >= 9:
            text = ','.join('{id:%s%s}' % (eid(k), (',' + n) if n else '') for k, n in riders)
            add(mount, 'Passengers:[%s]' % text + (',' + mount_nbt if mount_nbt else ''))
        else:
            form = entity_form(mount)
            mid = form[0] if form else mount
            rider, rnbt = riders[0]
            rform = entity_form(rider)
            add(rider, (rnbt + ',' if rnbt else '') + 'Riding:{id:%s%s%s}' % (q(mid), (',' + form[1]) if form and form[1] else '',
                                                                              (',' + mount_nbt) if mount_nbt else ''))

    # monsters
    for i, k in enumerate(['zombie', 'husk', 'skeleton', 'stray', 'wither_skeleton', 'creeper', 'spider', 'cave_spider', 'silverfish',
                           'endermite', 'blaze', 'ghast', 'witch', 'giant', 'guardian', 'elder_guardian', 'shulker', 'vex',
                           'evocation_illager', 'vindication_illager', 'illusion_illager', 'zombie_pigman', 'slime', 'magma_cube']):
        extra = equipment(i)
        if k == 'creeper':
            extra += ',powered:1b,Fuse:30s,ExplosionRadius:4b,ignited:0b'
        elif k in ('zombie', 'husk'):
            extra += ',IsBaby:1b,CanBreakDoors:1b'
        elif k == 'shulker':
            extra = 'Color:%db,Peek:30b,AttachFace:0b,APX:1,APY:2,APZ:3' % (i % 16)
        elif k == 'vex':
            extra += ',BoundX:1,BoundY:2,BoundZ:3,LifeTicks:100'
        elif k == 'zombie_pigman':
            extra += ',Anger:200s,HurtBy:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6"'
        elif k in ('slime', 'magma_cube'):
            extra += ',Size:2'
        elif k in ('ghast',):
            extra = 'ExplosionPower:3'
        elif k.endswith('illager') and k != 'evocation_illager':
            extra += ',Johnny:1b' if k == 'vindication_illager' else ''
        add(k, extra + ',' + EFFECTS + ',' + ATTRS + ',Health:%d.0f,CustomName:"%s",CustomNameVisible:%db,Tags:["scene","t%d"]' % (
            5 + i, k.title() + ' ' + str(i), i % 2, i))
    add('skeleton', 'SkeletonType:0b,' + equipment(3))
    add('zombie', 'ZombieType:0,' + equipment(4))
    add('zombie_villager', 'Profession:1,ConversionTime:-1,IsVillager:1b')
    for prof in range(6):
        add('zombie_villager', 'Profession:%d,ConversionTime:%d,IsBaby:%db,CustomName:"Zombie %d"' % (prof, [-1, 999999, -1][prof % 3], prof % 2, prof))
    if ERA > 8:
        add('zombie', 'IsVillager:1b,VillagerProfession:2')
    # animals
    for color in range(16):
        add('sheep', 'Color:%db,Sheared:%db,Age:%d,InLove:0,%s' % (color, color % 2, -24000 if color % 3 == 0 else 0, ATTRS if color == 3 else 'Health:8.0f'))
    for color in range(16):
        add('wolf', 'CollarColor:%db,Sitting:%db,Angry:%db,%s' % (color, color % 2, color % 5 == 0, 'Owner:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",OwnerUUID:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6"' if color % 2 else 'Health:8.0f'))
    for cat in range(4):
        add('ocelot', 'CatType:%d,Sitting:%db,Owner:"Dinnerbone"' % (cat, cat % 2))
    for r in (0, 1, 2, 3, 4, 5, 99):
        add('rabbit', 'RabbitType:%d,MoreCarrotTicks:%d' % (r, r * 3))
    for v in range(5):
        add('parrot', 'Variant:%d,Sitting:%db,OwnerUUID:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6"' % (v, v % 2))
    for kind in ('pig', 'cow', 'chicken', 'mooshroom', 'polar_bear', 'squid', 'bat', 'snowman', 'villager_golem'):
        extra = equipment(5) if kind not in ('bat', 'squid') else ''
        if kind == 'pig':
            extra = 'Saddle:1b,' + extra
        if kind == 'chicken':
            extra = 'IsChickenJockey:0b,EggLayTime:5000,' + extra
        if kind == 'bat':
            extra = 'BatFlags:1b'
        if kind == 'snowman':
            extra = 'Pumpkin:1b'
        if kind == 'villager_golem':
            extra = 'PlayerCreated:1b'
        add(kind, extra)
    # horses
    for v in (0, 1, 256, 513, 769, 1024, 1027, 4, 258):
        add('horse', 'Variant:%d,Temper:%d,Tame:1b,Bred:%db,EatingHaystack:0b,HasReproduced:0b,OwnerUUID:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",'
                     'ArmorItem:%s,SaddleItem:%s' % (v, 50 + v % 40, v % 2, item(['iron', 'golden', 'diamond'][v % 3] + '_horse_armor'), item('saddle')))
    add('donkey', 'ChestedHorse:1b,Items:%s,Tame:1b,SaddleItem:%s' % (slotted([item('wool', damage=3), item('apple'), item('dye', damage=15)], 2), item('saddle')))
    add('mule', 'ChestedHorse:1b,Items:%s,Tame:0b' % slotted([item('log', damage=1, count=5)], 2))
    add('skeleton_horse', 'SkeletonTrap:1b,SkeletonTrapTime:100,Tame:1b')
    add('zombie_horse', 'Tame:1b')
    for v in range(4):
        add('llama', 'Variant:%d,Strength:%d,Tame:1b,ChestedHorse:1b,DecorItem:%s,Items:%s,Temper:20'
            % (v, 1 + v, item('carpet', damage=v * 4 + 1), slotted([item('wool', damage=v), item('stone', damage=v)], 2)))
    # villagers
    for prof in range(6):
        for career in (1, 2):
            add('villager', 'Profession:%d,Career:%d,CareerLevel:%d,Riches:%d,Willing:%db,%s,Inventory:%s,Age:%d' % (
                prof, career, 1 + career, prof * 5, prof % 2, trades(3 + prof), '[%s]' % item('wheat_seeds', 4) if prof % 2 else '[]', -24000 if prof == 3 else 0))
    add('villager', 'Profession:0,Offers:{Recipes:[]}')
    # armor stands
    for i in range(10):
        add('armor_stand', 'ShowArms:%db,Small:%db,NoBasePlate:%db,Invisible:%db,Marker:%db,DisabledSlots:%d,CustomName:"Stand %d",%s,'
                           'Pose:{Head:[%d.0f,10.0f,0.0f],Body:[0.0f,%d.0f,5.0f],LeftArm:[-10.0f,0.0f,-10.0f],RightLeg:[1.0f,2.0f,3.0f]}'
            % (i % 2, (i // 2) % 2, i % 3 == 0, i % 4 == 0, i % 5 == 0, i * 64, i, equipment(i), i * 5, i * 3))
    # dropped items and orbs
    for i, s in enumerate([s for s in stacks.stacks if VALID_ITEMS is None or s.numeric is None][::37][:16]):
        add('item', 'Item:%s,Age:100,PickupDelay:32767,Health:5,Owner:"Upgrader",Thrower:"Upgrader"' % s)
    add('xp_orb', 'Value:50s,Age:100s,Health:5')
    # minecarts and boats
    add('minecart', 'CustomDisplayTile:1b,DisplayTile:"minecraft:stone",DisplayData:2,DisplayOffset:8,CustomName:"Cart"')
    add('chest_minecart', 'Items:%s,DisplayTile:"minecraft:chest",DisplayData:0,DisplayOffset:6' % stacks.items(20))
    add('chest_minecart', 'LootTable:"minecraft:chests/abandoned_mineshaft",LootTableSeed:7L')
    add('furnace_minecart', 'PushX:1.0d,PushZ:0.0d,Fuel:3000s')
    add('hopper_minecart', 'Items:%s,Enabled:0b,TransferCooldown:4' % stacks.items(5))
    add('tnt_minecart', 'TNTFuse:100', late=True)
    add('commandblock_minecart', 'Command:"say hello",LastOutput:%s,TrackOutput:1b,CustomName:"Commander"' % js({'text': '[@] hello'}))
    add('spawner_minecart', 'SpawnData:{id:"minecraft:pig"},SpawnPotentials:[{Entity:{id:"minecraft:cow"},Weight:3}],SpawnCount:2s,Delay:100s,RequiredPlayerRange:1s')
    for wood in WOODS:
        add('boat', 'Type:%s' % q(wood))
    ride('boat', '', [('villager', 'Profession:2'), ('pig', '')])
    # riding stacks
    ride('chicken', '', [('zombie', 'IsBaby:1b,' + equipment(2))])
    ride('spider', '', [('skeleton', (('HandItems:[%s,{}]' if ERA >= 9 else 'Equipment:[%s,{},{},{},{}]') % item('bow')))])
    ride('horse', 'Tame:1b', [('skeleton', 'Passengers:[{id:%s,Variant:3}]' % eid('parrot') if ERA >= 12 else '')])
    add('pig', 'Leashed:1b,Leash:{UUIDMost:%dL,UUIDLeast:%dL}' % (UUID_HI, 99))
    add('cow', 'Leashed:1b,Leash:{X:%d,Y:%d,Z:%d}' % (BX + 10, LAYERS[0], BZ + 10))
    add('enderman', 'carried:"minecraft:grass",carriedData:0s,' + equipment(1))
    add('enderman', 'carried:2s,carriedData:0s')
    add('enderman', 'carried:"minecraft:log",carriedData:3s')
    add('enderman', 'carried:35s,carriedData:5s')
    add('enderman', 'carried:"minecraft:wool",carriedData:11s')
    add('enderman', 'carried:"minecraft:stained_hardened_clay",carriedData:2s')
    # the last tick: things that move, expire, or need a wall
    for i, m in enumerate(['Kebab', 'Aztec', 'Alban', 'Aztec2', 'Bomb', 'Plant', 'Wasteland', 'Pool', 'Courbet', 'Sea', 'Sunset', 'Creebet',
                           'Wanderer', 'Graham', 'Match', 'Bust', 'Stage', 'Void', 'SkullAndRoses', 'Wither', 'Fighters', 'Pointer',
                           'Pigscene', 'BurningSkull', 'Skeleton', 'DonkeyKong']):
        add('painting', 'Motive:%s,Facing:%db' % (q(m), i % 4), late=True, stay=False)
    for i in range(10):
        add('item_frame', 'Facing:%db,ItemRotation:%db,ItemDropChance:%d.0f,Item:%s' % (i % 4, i % 8, 1, item(
            ['wool', 'stained_hardened_clay', 'dye', 'filled_map', 'log', 'skull', 'banner', 'potion', 'fish', 'spawn_egg'][i], damage=i + 3)), late=True, stay=False)
    add('item_frame', 'Facing:0b', late=True, stay=False)
    for i, (blk, data, te) in enumerate([('sand', 0, None), ('gravel', 0, None), ('anvil', 5, None), ('chest', 2, 'Items:[%s]' % item('wool', 1, 4, slot=0)),
                                          ('log', 6, None), ('concrete_powder' if ERA == 12 else 'sand', 11, None), ('stained_hardened_clay', 14, None),
                                          ('standing_sign', 3, 'Text1:%s' % js({'text': 'falling'}))]):
        add('falling_block', 'Block:"minecraft:%s",Data:%db,Time:1,DropItem:1b,HurtEntities:%db,FallHurtMax:40,FallHurtAmount:2.0f%s'
            % (blk, data, i % 2, (',TileEntityData:{%s}' % te) if te else ''), late=True)
    add('falling_block', 'TileID:12,Data:1b,Time:1,DropItem:1b', late=True)
    add('falling_block', 'TileID:35,Data:14b,Time:1', late=True)
    add('tnt', 'Fuse:80b', late=True)
    for i, (kind, extra) in enumerate([
            ('arrow', 'inTile:"minecraft:stone",inData:0b,inGround:1b,xTile:1,yTile:2,zTile:3,life:10s,shake:2b,pickup:1b,player:1b,damage:2.0d,crit:1b'),
            ('arrow', 'inTile:"minecraft:planks",inData:3b,inGround:1b,pickup:2b'),
            ('arrow', 'inTile:5s,inData:2b,inGround:1b,xTile:4,yTile:5,zTile:6'),
            ('arrow', 'inTile:35,inData:7b,inGround:1b'),
            ('spectral_arrow', 'inTile:"minecraft:log",inData:2b,inGround:1b,Duration:100'),
            ('arrow', 'Potion:"minecraft:strong_poison",CustomPotionEffects:[{Id:1b,Amplifier:0b,Duration:100}],Color:255'),
            ('potion', 'Potion:%s' % (item('splash_potion', tag='{Potion:"minecraft:poison"}') if ERA >= 9 else item('potion', damage=16388))),
            ('potion', 'Potion:%s,inTile:0' % (item('lingering_potion', tag='{Potion:"minecraft:long_regeneration"}') if ERA >= 9
                                               else item('potion', damage=16385))),
            ('potion', 'Potion:%s' % (item('splash_potion', damage=16453) if ERA >= 9 else item('potion', damage=16453))),
            ('egg', 'inTile:"minecraft:stone",inGround:1b'), ('snowball', 'inTile:"minecraft:dirt",inData:1b'),
            ('ender_pearl', 'inTile:"minecraft:stone"'), ('xp_bottle', ''), ('eye_of_ender_signal', ''),
            ('fireball', 'direction:[0.0d,-0.1d,0.0d],power:[0.0d,-0.1d,0.0d],ExplosionPower:2,inTile:"minecraft:netherrack"'),
            ('small_fireball', 'direction:[0.0d,0.0d,0.0d],power:[0.0d,0.0d,0.0d]'), ('wither_skull', 'Invulnerable:1b,direction:[0.0d,0.0d,0.0d]'),
            ('shulker_bullet', 'Steps:5,TXD:0.1d,TYD:0.0d,TZD:0.1d,Target:{L:5L,M:7L}'), ('llama_spit', ''), ('evocation_fangs', 'Warmup:5'),
            ('ender_crystal', 'ShowBottom:1b,BeamTarget:{X:%d,Y:%d,Z:%d}' % (BX, LAYERS[0], BZ)), ('ender_crystal', 'ShowBottom:0b'),
            ('fireworks_rocket', 'Life:0,LifeTime:5000,FireworksItem:%s' % item('fireworks', 1, 0, '{Fireworks:{Flight:1b,Explosions:[{Type:2b,Colors:[I;255]}]}}')),
            ('area_effect_cloud', 'Particle:"mobSpell",Radius:3.0f,Duration:600000,Age:3,WaitTime:20,ReapplicationDelay:20,DurationOnUse:0.0f,RadiusOnUse:0.0f,'
                                  'RadiusPerTick:0.0f,Color:16711680,Potion:"minecraft:strong_healing",Effects:[{Id:1b,Amplifier:1b,Duration:100}]'),
            ('area_effect_cloud', 'Particle:"blockcrack",ParticleParam1:1,ParticleParam2:0,Radius:2.0f,Duration:600000,Potion:"minecraft:water"'),
            ('area_effect_cloud', 'Particle:"iconcrack",ParticleParam1:35,ParticleParam2:3,Radius:1.0f,Duration:600000'),
            ('area_effect_cloud', 'Particle:"reddust",Radius:1.0f,Duration:600000,Owner:"Dinnerbone"')]):
        add(kind, extra, late=True)
    return E


def summon_all(scene, stacks, late):
    pk = Packer(scene.layers, stride=3, zstep=4)
    for i, (kind, nbt, is_late, stay) in enumerate(entity_list(stacks)):
        if is_late != late:
            continue
        form = entity_form(kind, i)
        if form is None:
            continue
        x, y, z = pk.cell()
        fields = uuid_tag(i + 1) + ',' + STAY
        if form[1]:
            fields += ',' + form[1]
        if nbt:
            fields += ',' + nbt
        if kind in ('painting', 'item_frame', 'leash_knot'):
            fields += ',TileX:%d,TileY:%d,TileZ:%d' % (x, y + 1, z)
            scene.summon(form[0], x, y + 1, z, fields, late)
        else:
            scene.summon(form[0], x + 0.5, y, z + 0.5, fields, late)


def entities(scene, stacks):
    summon_all(scene, stacks, False)


def late_content(scene, stacks):
    """Everything that is placed in the last tick, on platforms of its own (so that it can be wiped and redone)."""
    scene.layers.fresh()
    start = scene.layers.next
    transient_blocks(scene, stacks)
    summon_all(scene, stacks, True)
    scene.late_layers = LAYERS[start:scene.layers.next]
    scene.layers.fresh()


def wipe_late(scene):
    """Commands that remove the last tick's content again, if it has to be redone."""
    cmds = []
    for y in scene.late_layers:
        cmds.append(f'kill @e[type=!player,x={BX},y={y - 2},z={BZ},dx={SIZE},dy=14,dz={SIZE}]')
        for x in range(BX, BX + SIZE, 32):
            for z in range(BZ, BZ + SIZE, 32):
                cmds.append(f'fill {x} {y - 2} {z} {x + 31} {y + 11} {z + 31} minecraft:air 0')
        cmds.append(f'fill {BX} {y - 2} {BZ} {BX + SIZE - 1} {y - 1} {BZ + SIZE - 1} minecraft:stone 0')
    return cmds


# ------------------------------------------------------------------ player, scoreboard, advancements
PLAYER = 'Upgrader'
LAB = (BX + 56, 150, BZ + 107)  # a stone platform in the air where the bot works; (x, floor y, z)


def inventory_commands(stacks):
    cmds = []
    pool = [s for s in stacks.stacks if not s.numeric and s.name != 'air']
    # a hand-picked start of the hotbar, then the sweep
    hot = [item('map'), item('map'), item('map'), item('iron_hoe', 1, 249), item('stone', 64), item('flint_and_steel', 1, 63),
           item('wool', 64, 14), item('cobblestone', 32), item('torch', 20)]
    if 8 < ERA < 12:    # nobody uses the maps here: the server makes the map files of filled maps in a player's inventory
        hot[:3] = [item('filled_map', 1, i) for i in range(3)]
    for i, st in enumerate(hot):
        cmds.append(f'replaceitem entity {PLAYER} slot.hotbar.{i} {st.command_args()}')
    pool = [item('log', 16, 2), item('diamond_pickaxe', 1, 1560, '{%s}' % ench((32, 5), (34, 3)))] + pool
    for i in range(26):
        st = pool[(i * 41 + 7) % len(pool)] if i > 1 else pool[i]
        cmds.append(f'replaceitem entity {PLAYER} slot.inventory.{i} {st.command_args()}')
    armor = [('feet', item('leather_boots', 1, 12, '{%s}' % display('Boots', color=255))),
             ('legs', item('iron_leggings', 1, 20, '{%s}' % ench((0, 4), (34, 3)))),
             ('chest', item('elytra', 1, 100) if ERA >= 9 else item('diamond_chestplate', 1, 100)),
             ('head', item('skull', 1, 3, '{SkullOwner:{Id:"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",Name:"Dinnerbone",'
                                           'Properties:{textures:[{Value:"eyJ0ZXh0dXJlcyI6e319"}]}}}'))]
    for slot, st in armor:
        cmds.append(f'replaceitem entity {PLAYER} slot.armor.{slot} {st.command_args()}')
    if ERA >= 9:
        cmds.append(f'replaceitem entity {PLAYER} slot.weapon.offhand {item("shield", 1, 7, "{BlockEntityTag:{Base:11,Patterns:[{Pattern:%s,Color:3}]}}" % q("bs")).command_args()}')
    for i in range(27):
        st = pool[(i * 17 + 300) % len(pool)]
        cmds.append(f'replaceitem entity {PLAYER} slot.enderchest.{i} {st.command_args()}')
    return cmds


OBJECTIVES = [('dummy1', 'dummy', 'Dummy "quoted" objective'), ('kills', 'playerKillCount', 'Kills'), ('mined', 'stat.mineBlock.minecraft.stone', 'Mined stone'),
              ('crafted', 'stat.craftItem.minecraft.stick', None), ('usedsword', 'stat.useItem.minecraft.diamond_sword', 'Swung'),
              ('brokenpick', 'stat.breakItem.minecraft.iron_pickaxe', None), ('pickedup', 'stat.pickup.minecraft.dirt', None),
              ('dropped', 'stat.drop.minecraft.cobblestone', None), ('killedzombie', 'stat.killEntity.Zombie', 'Zombies'),
              ('killedbyskel', 'stat.entityKilledBy.Skeleton', None), ('walked', 'stat.walkOneCm', 'Walk'), ('jumps', 'stat.jump', None),
              ('kbt_red', 'killedByTeam.red', None), ('tk_blue', 'teamkill.blue', None), ('hp', 'health', 'Health'), ('foodlvl', 'food', None),
              ('airlvl', 'air', None), ('armorlvl', 'armor', None), ('xplvl', 'xp', None), ('levels', 'level', None), ('deaths', 'deathCount', 'Deaths'),
              ('totalkills', 'totalKillCount', None), ('trig', 'trigger', 'A trigger'), ('chest_open', 'stat.chestOpened', None),
              ('mine_log', 'stat.mineBlock.minecraft.log', None), ('killed_pig', 'stat.killEntity.Pig', None)]


def objectives():
    """The objectives this version has (1.8 lacks the food, air, armor, xp and level criteria and the pickup and drop statistics)."""
    if ERA >= 9:
        return OBJECTIVES
    return [o for o in OBJECTIVES if o[1] not in ('food', 'air', 'armor', 'xp', 'level') and not o[1].startswith(('stat.pickup', 'stat.drop'))]


def scoreboard_commands():
    cmds = []
    for name, crit, disp in objectives():
        cmds.append(f'scoreboard objectives add {name} {crit}' + (f' {disp}' if disp else ''))
    cmds += ['scoreboard objectives setdisplay sidebar kills', 'scoreboard objectives setdisplay list hp',
             'scoreboard objectives setdisplay belowName deaths', 'scoreboard objectives setdisplay sidebar.team.red mined',
             'scoreboard objectives setdisplay sidebar.team.dark_blue walked']
    colors = ['black', 'dark_blue', 'dark_green', 'dark_aqua', 'dark_red', 'dark_purple', 'gold', 'gray', 'dark_gray', 'blue', 'green',
              'aqua', 'red', 'light_purple', 'yellow', 'white']
    for i, c in enumerate(colors):
        cmds.append(f'scoreboard teams add t_{c} Team {c.replace("_", " ").title()}')
        cmds.append(f'scoreboard teams option t_{c} color {c}')
        cmds.append(f'scoreboard teams option t_{c} friendlyfire {"true" if i % 2 else "false"}')
        cmds.append(f'scoreboard teams option t_{c} seeFriendlyInvisibles {"true" if i % 3 else "false"}')
        cmds.append(f'scoreboard teams option t_{c} nametagVisibility {["always", "never", "hideForOtherTeams", "hideForOwnTeam"][i % 4]}')
        cmds.append(f'scoreboard teams option t_{c} deathMessageVisibility {["always", "never", "hideForOtherTeams", "hideForOwnTeam"][(i + 1) % 4]}')
        if ERA >= 9:
            cmds.append(f'scoreboard teams option t_{c} collisionRule {["always", "never", "pushOtherTeams", "pushOwnTeam"][i % 4]}')
    cmds += ['scoreboard teams join t_red ' + PLAYER, 'scoreboard teams join t_blue Fakeplayer1 Fakeplayer2',
             'scoreboard teams join t_gold Zombie']
    for i, (name, crit, disp) in enumerate(objectives()):
        if crit in ('health', 'food', 'air', 'armor', 'xp', 'level'):
            continue                      # read-only criteria
        cmds.append(f'scoreboard players set {PLAYER} {name} {i * 7 - 3}')
        cmds.append(f'scoreboard players set Fakeplayer{1 + i % 2} {name} {i}')
    tags = ['scoreboard players tag ' + PLAYER + ' add scene', 'scoreboard players tag ' + PLAYER + ' add second'] if ERA >= 9 else []
    cmds += ['scoreboard players enable ' + PLAYER + ' trig', *tags, 'scoreboard players add ' + PLAYER + ' kills 5',
             'scoreboard players operation ' + PLAYER + ' dummy1 = Fakeplayer1 dummy1']
    return cmds


def advancement_commands():
    biomes = ['forest', 'plains', 'ocean', 'river', 'taiga', 'roofed_forest', 'extreme_hills', 'swampland', 'beaches', 'savanna']
    mobs = ['zombie', 'skeleton', 'spider', 'creeper', 'enderman', 'blaze', 'witch']
    cmds = ['advancement grant ' + PLAYER + ' only minecraft:adventure/root',
            'advancement grant ' + PLAYER + ' only minecraft:story/root',
            'advancement grant ' + PLAYER + ' only minecraft:story/mine_stone',
            'advancement grant ' + PLAYER + ' only minecraft:husbandry/root',
            'advancement grant ' + PLAYER + ' until minecraft:story/upgrade_tools',
            'advancement grant ' + PLAYER + ' only minecraft:nether/root',
            'advancement grant ' + PLAYER + ' only minecraft:end/root']
    cmds += ['advancement grant %s only minecraft:adventure/adventuring_time %s' % (PLAYER, b) for b in biomes]
    cmds += ['advancement grant %s only minecraft:adventure/kill_all_mobs %s' % (PLAYER, m) for m in mobs]
    cmds += ['advancement grant %s only minecraft:husbandry/bred_all_animals %s' % (PLAYER, m) for m in ('pig', 'cow', 'sheep')]
    cmds += ['advancement grant %s only minecraft:husbandry/balanced_diet %s' % (PLAYER, m) for m in ('apple', 'bread', 'cooked_beef')]
    cmds += ['recipe give %s *' % PLAYER]
    return cmds


def achievement_commands():
    """Before 1.12: the achievements and the statistics are given by command."""
    return [f'achievement give * {PLAYER}'] + [f'achievement take achievement.{a} {PLAYER}' for a in ('theEnd2', 'overpowered', 'snipeSkeleton')]


def lab_commands():
    x, y, z = LAB
    cmds = [f'fill {x - 6} {y - 1} {z - 6} {x + 6} {y - 1} {z + 6} minecraft:stone 0',
            f'fill {x - 6} {y} {z - 6} {x + 6} {y + 4} {z + 6} minecraft:air 0']
    return cmds


# ------------------------------------------------------------------ the bot works in the lab: real statistics and maps
def lab_actions(server, bot, problems):
    """Everything here is done by the bot with real packets, so the server records genuine statistics."""
    x, y, z = LAB
    pause = time.sleep
    cx, cz = x + 0.5, z + 0.5
    say = lambda cmds: batch(server, cmds, problems)
    say(lab_commands() + [f'tp {PLAYER} {cx} {y} {cz}'])
    pause(1.5)
    # containers around the bot
    ring = [(-3, -3, 'chest', 2), (-2, -3, 'trapped_chest', 2), (-1, -3, 'ender_chest', 2), (0, -3, 'furnace', 2), (1, -3, 'crafting_table', 0),
            (2, -3, 'enchanting_table', 0), (3, -3, 'anvil', 0), (-3, 3, 'hopper', 0), (-2, 3, 'dispenser', 3), (-1, 3, 'dropper', 3),
            (0, 3, 'brewing_stand', 0), (1, 3, 'beacon', 0), (2, 3, 'white_shulker_box', 1), (3, 3, 'jukebox', 0), (-3, -1, 'noteblock', 0),
            (-3, 0, 'cake', 0), (-3, 1, 'cauldron', 1)]
    say([f'setblock {x + dx} {y} {z + dz} minecraft:{n} {d} replace' for dx, dz, n, d in ring])
    say([f'fill {x - 3} {y - 1} {z - 5} {x + 3} {y - 1} {z - 5} minecraft:grass 0',
         f'setblock {x - 3} {y} {z - 5} minecraft:red_flower 0 replace', f'setblock {x - 2} {y} {z - 5} minecraft:yellow_flower 0 replace',
         f'setblock {x - 1} {y} {z - 5} minecraft:tallgrass 1 replace', f'setblock {x} {y} {z - 5} minecraft:sapling 2 replace',
         f'setblock {x + 1} {y} {z - 5} minecraft:red_flower 3 replace', f'setblock {x + 2} {y} {z - 5} minecraft:double_plant 0 replace',
         f'setblock {x + 3} {y} {z - 5} minecraft:deadbush 0 replace'])
    pause(1)
    # open them all: right click with an empty hand
    bot.held(8)
    for dx, dz, n, d in ring:
        bot.place(x + dx, y, z + dz, 1)
        pause(0.25)
    bot.send(0x08, varint0(0))
    pause(0.5)
    # note block: hit it, the cake is eaten by the right click above
    bot.send(0x14, bytes([0]) + pos_long(x - 3, y, z - 1) + bytes([1]))
    pause(0.3)
    # walk to the plants and pick them: mined blocks, picked up items, walked distance
    for i in range(7):
        bot.move(x - 3 + i + 0.5, y, z - 4 + 0.5)
        pause(0.2)
        bot.dig(x - 3 + i, y, z - 5)
        pause(0.4)
    # walk, sneak, sprint and jump a little
    px, pz = x + 0.5, z - 3.5
    for i in range(24):
        px += 0.2
        bot.move(px, y, pz)
        pause(0.06)
    bot.send(0x15, varint0(3) + varint0(0) + varint0(0))
    for i in range(24):
        px -= 0.25
        bot.move(px, y, pz)
        pause(0.06)
    bot.send(0x15, varint0(4) + varint0(0) + varint0(0))
    bot.send(0x15, varint0(0) + varint0(0) + varint0(0))
    for i in range(10):
        px += 0.1
        bot.move(px, y, pz)
        pause(0.06)
    bot.send(0x15, varint0(1) + varint0(0) + varint0(0))
    for i in range(3):
        bot.send(0x0D, struct.pack('>dddB', px, y + 0.42, pz, 0))
        pause(0.15)
        bot.send(0x0D, struct.pack('>dddB', px, y, pz, 1))
        pause(0.4)
    bot.move(cx, y, cz)
    pause(0.3)
    # place blocks with the items in the hotbar
    for slot, target in ((4, (0, 1)), (6, (2, 1)), (7, (4, 1)), (8, (-2, 2))):
        bot.held(slot)
        pause(0.2)
        for k in range(3):
            bot.place(x + target[0] + k, y - 1, z + target[1] + 1, 1)
            pause(0.2)
    # a hoe used up, flint and steel used up
    bot.held(3)
    pause(0.2)
    bot.place(x - 3, y - 1, z - 5, 1)
    pause(0.4)
    bot.held(5)
    pause(0.2)
    bot.place(x + 4, y - 1, z, 1)
    pause(0.4)
    # drop an item
    bot.held(7)
    pause(0.2)
    bot.send(0x14, varint0(4) + pos_long(0, 0, 0) + bytes([0]))
    pause(0.3)
    # kill a few mobs
    for kind, tid in (('pig', 90), ('zombie', 54), ('skeleton', 51), ('cow', 92), ('sheep', 91)):
        say([f'summon {kind} {x + 1.5} {y} {z + 1.5} {{NoAI:1b,Health:1.0f,Tags:["labvictim"],PersistenceRequired:1b}}'])
        pause(0.6)
        eid = max([e for e, t in bot.entities.items() if t == tid], default=None)   # the newest one is ours
        if eid is not None:
            for _ in range(3):
                bot.attack(eid)
                pause(0.4)
        pause(0.3)
    say(['kill @e[tag=labvictim]'])
    # talk to a villager
    say([f'summon villager {x + 1.5} {y} {z - 1.5} {{NoAI:1b,Profession:1,Career:1,Tags:["labvictim"]}}'])
    pause(0.6)
    eid = max([e for e, t in bot.entities.items() if t == 120], default=None)
    if eid is not None:
        bot.send(0x0A, varint0(eid) + varint0(0) + varint0(0))
        pause(0.4)
    bot.send(0x08, varint0(1))
    say(['kill @e[tag=labvictim]'])
    # the maps: slot 0, 1 and 2 hold an empty map each
    for slot, (mx, mz) in enumerate(((x, z), (x - 80, z - 60), (x - 40, z - 20))):
        if slot:
            say([f'fill {mx - 2} {y - 1} {mz - 2} {mx + 2} {y - 1} {mz + 2} minecraft:stone 0', f'tp {PLAYER} {mx + 0.5} {y} {mz + 0.5}'])
            pause(1.0)
        bot.held(slot)
        pause(0.2)
        bot.use_item()
        pause(5 if slot == 0 else 3)
    bot.held(0)
    say([f'tp {PLAYER} {cx} {y} {cz}'])
    pause(1.5)
    # die once, killed by a creeper (keepInventory keeps the player's things), and come back
    say(['gamerule keepInventory true',
         f'summon creeper {cx} {y} {cz} {{ignited:1b,Fuse:1s,ExplosionRadius:6b,PersistenceRequired:1b,Tags:["labvictim"]}}'])
    pause(2.5)
    bot.respawn()
    pause(2.5)
    say(['gamerule keepInventory false', 'kill @e[tag=labvictim]',
         f'effect {PLAYER} minecraft:speed 99999 1 true', f'effect {PLAYER} minecraft:night_vision 99999 0 true',
         f'effect {PLAYER} minecraft:regeneration 99999 0 false', f'tp {PLAYER} {BX}.5 {LAYERS[0]} {BZ}.5'])
    pause(1.5)


def varint0(v):
    out = b''
    while True:
        b = v & 0x7F
        v >>= 7
        out += bytes([b | (0x80 if v else 0)])
        if not v:
            return out


# Slow commands at the start of the last batch: while the server executes them the rest of the batch arrives, so that
# it all runs in one tick (the server runs queued console commands until its list is empty).
def spawn_origin(server, say):
    """Where to build: around the world spawn, whose chunks stay loaded (96 x 96 blocks, on chunk borders)."""
    say(['save-all'])
    time.sleep(2)
    import anvil_dump
    level = anvil_dump.read_dat(str(server.directory / 'world' / 'level.dat'))['Data']
    return (level['SpawnX'] - 48) // 16 * 16, (level['SpawnZ'] - 48) // 16 * 16


def probe_items(server, say, names):
    """The item names this version knows (an unknown id is dropped when a chest loads its items)."""
    x, y, z = BX + 1, 150, BZ + 1
    cmds = [f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone 0']
    for n in names:
        cmds += [f'setblock {x} {y} {z} minecraft:chest 2 replace {{Items:[{{Slot:0b,id:"minecraft:{n}",Count:1b,Damage:0s}}]}}',
                 f'testforblock {x} {y} {z} minecraft:chest -1 {{Items:[{{id:"minecraft:{n}"}}]}}']
    out = batch(server, cmds, [], quiet=True)
    good = {n for n, o in zip(names, out[2::2]) if any('Successfully' in l for l in o)}
    batch(server, [f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y} {z + 1} minecraft:air 0'], [], quiet=True)
    return good


def barrier():
    return [f'fill {BX + dx} 108 {BZ + dz} {BX + dx + 31} 113 {BZ + dz + 31} minecraft:{b} 0'
            for b in ('stone', 'air', 'stone', 'air') for dx in (0, 32, 64) for dz in (0, 32, 64)]


def set_origin(bx, bz):
    """Move the whole scene (the flat world has its spawn chunks elsewhere)."""
    global BX, BZ, LAB
    BX, BZ = bx, bz
    LAB = (bx + 56, 150, bz + 107)


LAST = {}


def populate(server, Bot, port, protocol, name, problems):
    """Build the whole 1.12.2 scene, save it for good and return the (still joined) bot."""
    harness = sys.modules[Bot.__module__]
    Player = make_player(Bot, lab.varint, lab.parse_varint, harness.mcstring)
    set_protocol(protocol)
    say = lambda cmds, **kw: batch(server, cmds, problems, **kw)
    if ERA < 12:
        global VALID_ITEMS
        VALID_ITEMS = None
        origin = spawn_origin(server, say)
        set_origin(*origin)
        VALID_ITEMS = probe_items(server, say, ITEMS)
    stacks = Stacks()
    scene = Scene()
    build_blocks(scene, stacks)
    entities(scene, stacks)
    late_content(scene, stacks)
    LAST['scene'] = scene
    say(['gamerule randomTickSpeed 0', 'gamerule doDaylightCycle false', 'gamerule doWeatherCycle false', 'gamerule doMobSpawning false',
         'gamerule doFireTick false', 'gamerule mobGriefing false', 'gamerule sceneRule fixture', 'gamerule logAdminCommands true',
         *(['gamerule announceAdvancements false', 'gamerule spawnRadius 7'] if ERA == 12 else []), 'time set 6000', 'weather clear 999999', 'difficulty 2',
         'worldborder set 59999968', 'worldborder damage amount 0.25', 'worldborder warning distance 7'])
    say(scene.cmds)
    bot = Player(port, protocol, name)
    time.sleep(3)
    say(inventory_commands(stacks))
    say([f'xp 117L {name}', f'xp 29 {name}',
         *([f'effect {name} minecraft:speed 99999 1 true', f'effect {name} minecraft:night_vision 99999 0 true'] if ERA == 12 else
           [f'effect {name} 1 99999 1 true', f'effect {name} 16 99999 0 true']),
         f'spawnpoint {name} {BX} {LAYERS[0]} {BZ}', f'gamemode survival {name}'])
    say(scoreboard_commands())
    if ERA == 12:
        say(advancement_commands())
        try:
            lab_actions(server, bot, problems)
        except OSError as error:      # the server dropped the bot
            problems.append(('lab_actions', ['the bot lost its connection: %s' % error]))
    else:
        say(achievement_commands())
        if ERA == 8:                  # 1.8 makes a map only when an empty map is used
            for slot in range(3):
                bot.use_map_18(slot)
            bot.send(0x09, struct.pack('>h', 8))
        time.sleep(3)                 # the inventory ticks once: the server creates the maps
    time.sleep(2)
    say(['save-all flush'])
    bot.leave()            # the player's data, statistics and advancements are written when the bot leaves
    time.sleep(2)
    # The last tick: transient states and entities, then the save. A normal stop would save the chunks again,
    # after those states have changed (it ignores `save-off`), so the server is killed once the save is done.
    # `time query gametime` before and after tells whether a tick ran in between (the server executes a queue of
    # console commands in one tick unless the queue runs dry); if so the content is wiped and placed again.
    slow = barrier()
    batch_commands = ['time query gametime'] + slow + scene.late + ['save-all flush', 'time query gametime']
    for attempt in range(5):
        kept = len(problems)
        if attempt:
            say(wipe_late(scene))
        out = say(batch_commands, chunk=len(batch_commands) + 5)
        times = [m.group(1) for o in (out[0], out[-1]) for l in o for m in [re.search(r'[Tt]ime is (\d+)', l)] if m]
        if len(times) == 2 and times[0] == times[1]:
            break
        del problems[kept:]
        print(f'  the last tick was split ({times}), again')
    else:
        problems.append(('last tick', ['the transient states were not saved in one tick']))
    time.sleep(1)
    server.process.kill()
    server.process.wait()
    try:
        server.process.stdin.close()
    except OSError:
        pass
    return bot


# ------------------------------------------------------------------ after the server stopped
HAND_STATS = {
    # HAND-WRITTEN FIXTURE: statistics that the passive bot cannot cause (they are added only where the real
    # server's file lacks the key), in the exact key format of a 1.12.2 stats file.
    'stat.craftItem.minecraft.planks': 12, 'stat.craftItem.minecraft.stick': 8, 'stat.craftItem.minecraft.crafting_table': 1,
    'stat.craftItem.minecraft.wooden_pickaxe': 1, 'stat.craftItem.minecraft.spawn_egg': 2, 'stat.craftItem.minecraft.dye': 3,
    'stat.craftItem.minecraft.stained_hardened_clay': 4, 'stat.useItem.minecraft.stone': 40, 'stat.useItem.minecraft.log': 6,
    'stat.useItem.minecraft.wooden_pickaxe': 17, 'stat.useItem.minecraft.spawn_egg': 3, 'stat.useItem.minecraft.bed': 1,
    'stat.mineBlock.minecraft.stone': 55, 'stat.mineBlock.minecraft.log': 14, 'stat.mineBlock.minecraft.log2': 3,
    'stat.mineBlock.minecraft.dirt': 31, 'stat.mineBlock.minecraft.grass': 9, 'stat.mineBlock.minecraft.leaves': 22,
    'stat.mineBlock.minecraft.coal_ore': 4, 'stat.mineBlock.minecraft.iron_ore': 2, 'stat.mineBlock.minecraft.wool': 3,
    'stat.mineBlock.minecraft.stained_hardened_clay': 1, 'stat.mineBlock.minecraft.double_plant': 2,
    'stat.breakItem.minecraft.wooden_pickaxe': 1, 'stat.breakItem.minecraft.diamond_pickaxe': 1, 'stat.breakItem.minecraft.shears': 1,
    'stat.breakItem.minecraft.spawn_egg': 1, 'stat.pickup.minecraft.dirt': 31, 'stat.pickup.minecraft.log': 14, 'stat.pickup.minecraft.cobblestone': 50,
    'stat.pickup.minecraft.spawn_egg': 1, 'stat.pickup.minecraft.dye': 2, 'stat.drop.minecraft.cobblestone': 5, 'stat.drop.minecraft.dirt': 2,
    'stat.drop.minecraft.spawn_egg': 1, 'stat.killEntity.Zombie': 7, 'stat.killEntity.Skeleton': 3, 'stat.killEntity.Creeper': 1,
    'stat.killEntity.PigZombie': 1, 'stat.killEntity.LavaSlime': 2, 'stat.killEntity.MushroomCow': 1, 'stat.killEntity.Ozelot': 1,
    'stat.killEntity.ZombieVillager': 1, 'stat.killEntity.EvocationIllager': 1, 'stat.entityKilledBy.Skeleton': 1, 'stat.entityKilledBy.Zombie': 2,
    'stat.entityKilledBy.Creeper': 1, 'stat.entityKilledBy.Spider': 1, 'stat.entityKilledBy.VindicationIllager': 1,
    'stat.walkOneCm': 8240, 'stat.crouchOneCm': 310, 'stat.sprintOneCm': 4500, 'stat.swimOneCm': 120, 'stat.fallOneCm': 1500,
    'stat.climbOneCm': 220, 'stat.flyOneCm': 90, 'stat.diveOneCm': 60, 'stat.minecartOneCm': 700, 'stat.boatOneCm': 400, 'stat.pigOneCm': 30,
    'stat.horseOneCm': 1200, 'stat.aviateOneCm': 800, 'stat.jump': 66, 'stat.damageDealt': 410, 'stat.damageTaken': 188, 'stat.deaths': 3,
    'stat.mobKills': 17, 'stat.animalsBred': 4, 'stat.playerKills': 0, 'stat.fishCaught': 2, 'stat.talkedToVillager': 5,
    'stat.tradedWithVillager': 2, 'stat.cakeSlicesEaten': 2, 'stat.cauldronFilled': 1, 'stat.cauldronUsed': 3, 'stat.armorCleaned': 1,
    'stat.bannerCleaned': 2, 'stat.brewingstandInteraction': 4, 'stat.beaconInteraction': 1, 'stat.dropperInspected': 1,
    'stat.hopperInspected': 2, 'stat.dispenserInspected': 3, 'stat.noteblockPlayed': 6, 'stat.noteblockTuned': 4, 'stat.flowerPotted': 2,
    'stat.trappedChestTriggered': 1, 'stat.enderchestOpened': 3, 'stat.itemEnchanted': 2, 'stat.recordPlayed': 1,
    'stat.furnaceInteraction': 9, 'stat.craftingTableInteraction': 12, 'stat.chestOpened': 15, 'stat.sleepInBed': 2, 'stat.shulkerBoxOpened': 1,
    'stat.sneakTime': 400,
}


def fixtures(world):
    """Add what a real server could not be made to produce (see HAND_STATS), clearly apart from the real data."""
    world = Path(world)
    for f in sorted((world / 'stats').glob('*.json')):
        data = json.loads(f.read_text())
        for key, value in HAND_STATS.items():
            data.setdefault(key, value)
        f.write_text(json.dumps(data, separators=(',', ':')))
        print(f'  stats fixture: {len(data)} counters in {f.name}')


# ------------------------------------------------------------------ the flat world
FLAT_OPTIONS = '3;7,2x3,2;1;village,decoration'     # 1.12 style: version;layers;biome;features


def build_flat(h, force=False):
    """`1.12.2-flat`: the same scene on a flat world (generator flat, the settings above), built with the harness
    module `h` (tools/datafix_worlds.py)."""
    version = '1.12.2'
    jar, data_version, protocol = h.server_jar(version)
    target = h.SHARED / 'worlds' / (version + '-flat')
    if target.exists() and not force:
        print(f'{version}-flat: kept {target}')
        return []
    work = h.LOCAL / 'gen' / (version + '-flat')
    shutil.rmtree(work, ignore_errors=True)
    port = lab.free_port()

    class FlatServer(h.Server):
        def __init__(self, directory):         # as Server.__init__, with a flat world
            directory.mkdir(parents=True, exist_ok=True)
            (directory / 'eula.txt').write_text('eula=true\n')
            props = {'server-port': port, 'online-mode': 'false', 'level-name': 'world', 'level-seed': '4242',
                     'view-distance': '4', 'spawn-protection': '0', 'enable-rcon': 'false', 'enable-query': 'false',
                     'max-tick-time': '-1', 'level-type': 'FLAT', 'generator-settings': FLAT_OPTIONS}
            (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
            self.process = h.subprocess.Popen([h.java(version), '-Xmx2G', '-jar', str(jar), 'nogui'], cwd=directory,
                                              stdin=h.subprocess.PIPE, stdout=h.subprocess.PIPE, stderr=h.subprocess.STDOUT, text=True)
            self.lines = []
            self.queue = queue.Queue()
            self.counter = 0
            threading.Thread(target=self._pump, daemon=True).start()
            self.wait('Done (', 600)

    server = FlatServer(work)
    problems = []
    old = (BX, BZ)
    set_origin(16, 16)
    try:
        # A flat world's spawn is found far away; make it (64, 64), and let a bot visit the scene's area so that its chunks
        # are loaded (they stay loaded, being within the spawn radius).
        batch(server, ['setworldspawn 64 4 64'], problems)
        Player = make_player(h.Bot, lab.varint, lab.parse_varint, h.mcstring)
        visitor = Player(port, protocol, h.BOT)
        time.sleep(2)
        for x, z in ((24, 24), (24, 72), (24, 104), (72, 24), (72, 72), (72, 104), (104, 24), (104, 72), (104, 104), (72, 123), (-8, 63), (32, 103)):
            batch(server, [f'tp {h.BOT} {x}.5 4 {z}.5'], problems)
            time.sleep(1.5)
        batch(server, [f'tp {h.BOT} 64.5 4 64.5'], problems)
        time.sleep(1)
        visitor.leave()
        time.sleep(1)
        bot = populate(server, h.Bot, port, protocol, h.BOT, problems)
        bot.leave()
    finally:
        set_origin(*old)
        server.stop()
    fixtures(work / 'world')
    partial = target.with_name(target.name + '.partial')
    shutil.rmtree(partial, ignore_errors=True)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(work / 'world', partial)
    (partial / 'datafix-source.json').write_text(json.dumps(
        {'version': version, 'data_version': data_version, 'problems': problems, 'flat': FLAT_OPTIONS}, indent=1))
    shutil.rmtree(target, ignore_errors=True)
    partial.rename(target)
    for command, out in problems:
        print(f'{version}-flat: command failed: {command}\n   {out}')
    print(f'{version}-flat: DataVersion {data_version}, world in {target}')
    return problems
