"""Command corpus for tools/test_command_parse_parity.py.

The corpus walks vanilla's exported command tree. For every executable path it
emits the valid command (sample values per argument type), every token-boundary
prefix (completion suggestions), mutations that replace one argument by a bad
sample of its type (error message and cursor), a truncated last argument and a
trailing-garbage variant. Hand-written extra commands cover selectors, NBT
paths, coordinates, ranges and the other parsers in more depth.
"""
import hashlib

SELECTORS = [
    '@s', '@a', '@e', '@p', '@r', '@n', 'Player', 'Steve', 'some_name', '00000000-0000-0000-0000-000000000001',
    '@a[distance=..5]', '@e[distance=1..5]', '@e[distance=2]', '@e[type=minecraft:pig,limit=1,sort=nearest]', '@e[type=!zombie]',
    '@e[type=#minecraft:skeletons]', '@e[tag=a,tag=!b]', '@e[tag=]', '@e[tag=!]', '@e[team=red]', '@e[team=!red]', '@e[team=]',
    '@e[scores={kills=1..}]', '@e[scores={kills=1,health=..5}]', '@e[nbt={Invulnerable:1b}]', '@e[nbt=!{Invulnerable:1b}]',
    '@e[x=1,y=2,z=3,dx=4,dy=5,dz=6]', '@e[x=1.5]', '@e[level=5..10]', '@e[level=3]', '@a[gamemode=survival]', '@a[gamemode=!creative]',
    '@e[name="Bob"]', '@e[name=Bob]', '@e[name=!Bob]', '@e[advancements={minecraft:story/root=true}]',
    '@e[advancements={minecraft:story/root={a=true}}]', '@e[predicate=minecraft:foo]', '@e[predicate=!minecraft:foo]',
    '@e[x_rotation=..0]', '@e[y_rotation=90..180]', '@e[x_rotation=0]', '@e[limit=0]', '@e[limit=3]', '@e[sort=random]',
    '@e[sort=furthest]', '@e[sort=arbitrary]', '@e[type=player]', '@a[type=player]', '@e[distance=..5,limit=1]', '@e[ ]',
    '@e[ type = pig ]', '@e[type=pig , limit=2]', '@e[type=pig,]', '@e[,]', '@a[ name = "x" ]',
    '@a[scores={}]', '@a[advancements={}]', '@e[nbt={}]', '@e[nbt={a:1,b:[1,2]}]',
    '@e[x=1,x=2]', '@e[limit=1,limit=2]', '@s[type=pig]', '@e[type=pig,type=cow]', '@e[type=!pig,type=!cow]',
    '@e[distance=..5,distance=1..]', '@e[sort=nearest,sort=random]', '@e[gamemode=survival,gamemode=creative]',
]
BAD_SELECTORS = [
    '@x', '@', '@E', '@e[', '@e[]', '@e[distance=-1]', '@e[distance=..-1]', '@e[unknown=1]', '@e[limit=a]', '@e[limit=-1]',
    '@e[type=pig', '@e[type=minecraft:nope]', '@e[type=nope]', '@e[type=#nope]', '@e[sort=bad]', '@e[gamemode=bad]',
    '@e[nbt={]', '@e[nbt=]', '@e[scores={a=}]', '@e[scores={a=x}]', '@e[scores={a}]', '@e[scores={a=1', '@e[scores=1]',
    '@e[advancements={a=}]', '@e[advancements={nope=true}]', '@e[advancements={minecraft:story/root=maybe}]',
    '@e[level=a]', '@e[level=-1..]', '@e[x=]', '@e[x=a]', '@e[dx=a]', '@e[x_rotation=a]', '@e[name=]', '@e[name="x]',
    '@e[team]', '@e[tag]', '@e[limit]', '@e[=1]', '@e[a=1]', '@e[type=pig,]x', '@e[type=pig]x', '@e [type=pig]',
    '@e[predicate=]', '@e[predicate=minecraft:]', '@e[predicate=Bad]', '@e[type=Bad]', '@e[gamemode=]', '@e[sort=]',
    '@e[distance=..]', '@e[distance=1..a]', '@e[level=..]', '@e[nbt={a:}]', '@e[nbt=1]', '@a[limit=0]',
    '@e[nbt={a:1}', '@e[type=pig,type=!pig]', '@e[type=!pig,type=pig]', '@p[type=zombie]', '@a[type=zombie]', '@r[type=!player]',
    '@e[scores={kills=1..}', '@e[tag=a b]', '"Bob', '@e[type="pig"]', '@e[type=\'pig\']', '@s[', '@s]', '@e{}',
    'Bad Name', 'x'.ljust(17, 'y'), '\u00e9', '00000000-0000-0000-0000-00000000000g', '00000000-0000-0000-0000-00000000000',
]
COMPONENTS = [
    '"text"', '{"text":"hi"}', '[{"text":"a"},"b"]', '{"translate":"chat.type.text","with":["a","b"]}',
    '{"text":"x","color":"red","bold":true}', 'true', '12', '1.5', '{"selector":"@a"}', '{"selector":"@a","separator":", "}',
    '{"score":{"name":"@s","objective":"kills"}}', '{"nbt":"foo","entity":"@s"}', '{"nbt":"foo","block":"0 0 0"}',
    '{"nbt":"foo","storage":"a:b","interpret":true}', '{"keybind":"key.jump"}', "'single'", '{text:"snbt"}', '{text:"a",extra:["b",{text:"c"}]}',
    '{"text":"x","hover_event":{"action":"show_text","value":"y"}}', '{"text":"x","click_event":{"action":"open_url","url":"https://a.b"}}',
    '{"text":"x","click_event":{"action":"run_command","command":"/help"}}', '{"text":"x","font":"minecraft:uniform"}',
    '{"text":"x","color":"#ff00ff"}', '{"text":"x","shadow_color":-1}', '{"text":"x","insertion":"y"}', '[]', '[1,2]', '""', '{}',
    '{"type":"text","text":"x"}', '{"type":"translatable","translate":"x"}', '{"translate":"x","fallback":"y"}',
    '{"object":"atlas","sprite":"item/porkchop"}', '{"object":"player","player":{"name":"Notch"}}',
    '{text:"x",color:red}', '"a\\nb"', '{"text":"a","italic":false,"underlined":true,"strikethrough":false,"obfuscated":false}',
]
BAD_COMPONENTS = [
    '{"text":', '{"text":"x",}', '{"color":"nope","text":"x"}', '{"translate":}', '[', '{"text":"x","extra":5}', '{"unknown":1}',
    '{"text":"x"} y', '"unterminated', "{'a':1}", '{"text":1,"color":}', '{"selector":"@x"}', '{"nbt":"a"}',
    '{"score":{"name":"@s"}}', '{"keybind":5}', '{"text":"x","click_event":{"action":"nope"}}', '{"text":"x","bold":"maybe"}',
    '{"translate":"x","with":5}', '{"text":"x","color":"#gggggg"}', '{"text":"x","hover_event":{"action":"show_text"}}', 'nope',
    '{"text":"x","font":"Bad Font"}', '{"nbt":"a","entity":"@x"}', '{"type":"nope"}', '[{"text":"a"},', '{"text":"a"', '}',
]
NBT_COMPOUNDS = [
    '{}', '{a:1}', '{a:1b,b:"x",c:[1,2,3]}', '{a:{b:{c:1}}}', '{"quoted key":1}', "{'single':1}", '{a:[I;1,2,3]}', '{a:[B;1b,2b]}',
    '{a:[L;1L,2L]}', '{a:1.5f,b:2d,c:3L,d:4s}', '{a:1,}', '{ a : 1 }', '{a:true,b:false}', '{a:1.5e3}', '{a:0x10}', '{a:0b101}',
    '{a:-1b}', '{a:+1}', '{a:"\\n"}', '{a:[]}', '{a:[{}]}', '{a:[[1],[2]]}', '{a:1_000}', '{a:bare}', '{a:.5}', '{a:5.}',
    '{a:1,a:2}', '{a:[I;]}', '{a:[B;]}', '{a:1f}', '{a:1F}', '{a:1D}', '{a:NaN}', '{a:Infinity}', '{a:"\\u00e9"}',
]
BAD_NBT_COMPOUNDS = [
    '{', '{a:}', '{a:1', '{a 1}', '{a:[I;1b]}', '{a:[1,"x"]}', '{a:1,,}', '{,}', '{:1}', '{a:[B;1]}', '[]', '1', '"x"',
    '{a:"unterminated}', '{a:[}', '{a:1b2}', '{a:300b}', '{a:99999999999}', '{a:[I;1.5]}', '{a:1 b:2}', '{a:{}', '{a:1}x',
    '{"":1}', '{a:\\}', '{a:"\\x"}', '{a:0x}', '{a:1.2.3}', '{a:--1}',
]
NBT_TAGS = ['1', '1b', '"str"', '[1,2]', '{a:1}', 'true', '1.5', '[B;1b]', '1s', '1L', '1.5f', '2d', "'x'", 'bare', '[]', '[I;1]', '0', '-1']
BAD_NBT_TAGS = ['', '[', '{', '"x', '[1,"a"]', '1b2', '[B;1', '1.5.5', '[I;1b]', '1,', '{a}', ']']
NBT_PATHS = [
    'foo', 'foo.bar', 'foo[0]', 'foo[]', 'foo[{a:1}]', '{a:1}', 'foo{a:1}', '"quoted key".x', 'a.b[2].c', '[0]', 'foo[-1]', 'a[0][1]',
    'a[0].b', 'a{b:1}.c', '"a b"', "'a b'", 'a.b.c.d', '{}', 'a[{}]', 'a[ 0 ]', 'a.', 'Items[{Slot:0b}].id', 'a[{b:{c:1}}]',
    '_x', 'A1', '1a', 'a-b', 'a:b', 'a/b', 'a+b', '\u00e9',
]
BAD_NBT_PATHS = ['.', 'foo.', 'foo[', 'foo[a]', 'foo..bar', '{a:', 'foo[{a:1}', '[]foo', 'foo[0', 'foo]', 'foo[1.5]', 'foo[-]', '"foo', 'foo."bar', 'a{', '{a:1}{b:2}', 'a.{b:1}', '[{a:1}]x']
BLOCK_POS = ['0 64 0', '~ ~ ~', '~1 ~-2 ~3', '^ ^ ^1', '1 2 3', '~.5 ~ ~', '-1 -2 -3', '1.5 2 3', '~1.5 ~ ~', '^1.5 ^ ^', '0 0 0', '+1 +2 +3', '.5 1 1', '1e2 2 3']
BAD_BLOCK_POS = ['0 64', '~ ~', '^ ^ 1', '~1 ^2 3', 'a b c', '1 2', '~ a ~', '^ ~ ^', '1 2 ^', '99999999999 0 0', '~a ~ ~', '~ ~ ~x', '^1 ^2', '~-', '- 0 0']
VEC3 = ['0 64 0', '~ ~ ~', '1.5 2.5 -3.5', '^1 ^ ^-2', '.5 .5 .5', '1 2 3', '~0.5 ~ ~', '-0 0 0', '1e3 0 0', '+1 0 0', '1.5e-3 0 0']
BAD_VEC3 = ['1 2', '^ ~ ~', '~a ~ ~', '1.5 2', 'a b c', '~ ~', 'NaN 0 0', '1 2 ^', '0 0 0x', '1.2.3 0 0', '~- 0 0']
VEC2 = ['0 0', '~ ~', '1.5 -2', '~1 ~-1', '1 2']
BAD_VEC2 = ['1', '~ a', '^ ^', '1 ^', 'a b', '~ ~ ~']
COLUMN_POS = ['0 0', '~ ~', '~3 ~-4', '1 2', '-5 7']
BAD_COLUMN_POS = ['0', '~ ^', 'a b', '1.5 2', '^ ^', '0 0 0']
ROTATION = ['0 0', '~ ~', '90 -45', '~10 ~-5', '1.5 2.5', '-180 180', '^ ^']
BAD_ROTATION = ['^ ^', 'a b', '0', '~ a', '1 2 3']
INT_RANGES = ['1', '1..5', '..5', '1..', '0..0', '-5..-1', '-5..5', '5..5', '+1', '1..+5', '0']
BAD_INT_RANGES = ['..', '1...5', 'a', '5..1', '1..a', '1.5', '1..5..6', '-', '..-', ' ', '99999999999', '1..99999999999']
FLOAT_RANGES = ['1', '1.5', '1..5', '..5.5', '0.1..', '-1.5..2.5', '.5', '5.', '1..1', '1.5..1.5', '..0', '0..']
BAD_FLOAT_RANGES = ['..', 'a', '1.5.5', '5..1', '1..a', '1...2', '1e5', '-', '..-', '2..1.5']
TIMES = ['1', '1d', '1s', '1t', '0.5d', '2.5s', '100', '1.5t', '0', '0d', '3600', '24000', '1e2']
BAD_TIMES = ['1x', 'abc', '-1', '1.5.5s', '1 s', 'd', '99999999999', '1e', '.d', '-1t', '0.5t', '2147483648']
RESOURCE_LOCATIONS = ['minecraft:stone', 'stone', 'foo:bar', 'a/b', 'foo:bar/baz', 'foo.bar:a-b_c', 'a_b', '.', 'minecraft:']
BAD_RESOURCE_LOCATIONS = ['Stone', 'foo:Bar', 'a b', ':x', 'minecraft::x', 'a:b:c', 'foo:bar$', '\u00e9', '@a', '#a']
ITEM_STACKS = [
    'minecraft:stone', 'stone', 'minecraft:diamond_sword[damage=5]', 'minecraft:stone[]', 'minecraft:stone[minecraft:custom_name="x"]',
    'minecraft:stone[!damage]', 'minecraft:diamond_sword[damage=5,max_damage=10]', 'minecraft:stone[count=1]', 'minecraft:stone[ ]',
    'minecraft:stone[custom_name={"text":"x"}]', 'minecraft:stone[lore=["a","b"]]', 'minecraft:stone[enchantments={"minecraft:sharpness":1}]',
    'minecraft:stone[custom_data={a:1}]', 'minecraft:stone[unbreakable={}]', 'minecraft:stone[max_stack_size=1]', 'minecraft:stone[rarity=epic]',
    'minecraft:stone[!damage,!max_damage]', 'minecraft:stone[damage=5,!max_damage]', 'minecraft:player_head[profile="Notch"]', 'minecraft:stone[ damage = 5 ]',
    'minecraft:stone[repair_cost=0]', 'minecraft:stone[item_name="x"]', 'minecraft:stone[tooltip_display={hide_tooltip:true}]',
]
BAD_ITEM_STACKS = [
    'minecraft:nope', 'nope', 'minecraft:stone[', 'minecraft:stone[damage]', 'minecraft:stone[damage=x]', 'minecraft:stone[nope=1]',
    'minecraft:stone[damage=5,damage=6]', 'minecraft:stone{a:1}', 'minecraft:stone[!nope]', 'minecraft:stone[damage=5', 'minecraft:stone[damage=5,]',
    'minecraft:stone[=5]', 'minecraft:stone[!]', 'minecraft:stone[!damage=5]', 'minecraft:stone]', 'Minecraft:stone', 'minecraft:stone[damage=-5]',
    'minecraft:stone[damage=5,!damage]', 'minecraft:stone[max_stack_size=0]', 'minecraft:stone[max_stack_size=100]', 'minecraft:stone[rarity=nope]',
    'minecraft:stone[custom_name={"text":}]', 'minecraft:stone[enchantments=1]', '#minecraft:logs', '*', 'minecraft:stone[unknown:thing=1]',
]
ITEM_PREDICATES = [
    'minecraft:stone', 'stone', '*', '#minecraft:planks', '#planks', 'minecraft:stone[damage=5]', 'minecraft:stone[!damage]', 'stone[count=1]',
    'stone[count~1]', 'stone[damage|max_damage=5]', 'stone[damage~5]', 'stone[count={min:1}]', 'stone[custom_data~{a:1}]', '*[damage=5]',
    '#minecraft:planks[damage=5]', 'stone[enchantments~[{enchantments:"minecraft:sharpness"}]]', 'stone[enchantments~[{levels:{min:1}}]]',
    'stone[damage|count=1]', 'stone[count=1|damage=2]', 'stone[(count=1|damage=2),!max_damage]', 'stone[damage={durability:5}]',
]
BAD_ITEM_PREDICATES = [
    'minecraft:nope', '#nope', '#', 'stone[', 'stone[damage=x]', 'stone[nope=1]', 'stone[nope~1]', 'stone[count~]', 'stone[!nope]', 'stone[damage|]',
    'stone[|damage]', 'stone[damage||count]', '*[', '*x', 'stone[damage=5,damage=6]', 'stone[damage=1 count=2]', 'stone[count=1,]', 'stone[ ]x',
    'stone[count~a]', 'stone[(damage=1]', 'stone[damage=1)]',
]
BLOCK_STATES = [
    'minecraft:stone', 'stone', 'oak_stairs[facing=north]', 'oak_stairs[facing=north,half=top]', 'minecraft:chest{Items:[]}', 'stone{}', 'oak_stairs[]',
    'oak_stairs[ ]', 'oak_stairs[ facing = north ]', 'minecraft:chest[facing=west]{Items:[]}', 'oak_stairs[waterlogged=true,shape=straight]',
    'minecraft:oak_stairs[half=bottom,facing=south,shape=inner_left,waterlogged=false]', 'redstone_wire[north=side]', 'sculk_sensor[power=15]', 'cake[bites=0]',
]
BAD_BLOCK_STATES = [
    'minecraft:nope', 'oak_stairs[', 'oak_stairs[facing=up]', 'oak_stairs[nope=1]', 'oak_stairs[facing]', 'oak_stairs[facing=north,facing=south]', 'oak_stairs[facing=]',
    'oak_stairs[=north]', 'oak_stairs[facing=north', 'oak_stairs[facing=north,]', 'oak_stairs[facing=north]x', 'oak_stairs{', 'stone[a=b]', 'oak_stairs[facing=north]{}x',
    'oak_stairs]', '#minecraft:logs', 'oak_stairs[facing=North]', 'cake[bites=99]', 'minecraft:chest{', 'Minecraft:stone', 'stone{a:}',
]
BLOCK_PREDICATES = BLOCK_STATES + ['#minecraft:logs', '#minecraft:logs[axis=y]', '#logs', '#minecraft:logs{}', '#minecraft:logs[nope=1]']
BAD_BLOCK_PREDICATES = BAD_BLOCK_STATES[:-1] + ['#nope', '#', '#minecraft:logs[', '#minecraft:logs[axis=]', '#minecraft:logs[axis]', '#minecraft:logs[axis=y,axis=x]', '#minecraft:logs[axis=q]']
PARTICLES = [
    'flame', 'minecraft:flame', 'minecraft:dust{color:[1.0,0.0,0.0],scale:1.0}', 'minecraft:dust{color:16711680,scale:1.0}', 'minecraft:block{block_state:"minecraft:stone"}',
    'minecraft:block{block_state:{Name:"minecraft:stone"}}', 'minecraft:entity_effect{color:[1.0,0.0,0.0,1.0]}', 'minecraft:item{item:"minecraft:stick"}',
    'minecraft:item{item:{id:"minecraft:stick",count:1}}', 'minecraft:sculk_charge{roll:0.5}', 'minecraft:shriek{delay:5}', 'minecraft:dragon_breath{power:1.0}',
    'minecraft:trail{target:[0.0,0.0,0.0],color:16777215,duration:5}', 'minecraft:vibration{destination:{type:"block",pos:[0,0,0]},arrival_in_ticks:5}',
    'minecraft:dust_color_transition{from_color:[1.0,0.0,0.0],to_color:[0.0,0.0,1.0],scale:1.0}', 'minecraft:flame{}',
]
BAD_PARTICLES = ['nope', 'minecraft:dust', 'minecraft:dust{', 'minecraft:flame{a:1}x', 'minecraft:block', 'minecraft:block{}', 'minecraft:dust{color:1}', 'Flame', 'minecraft:']
ITEM_SLOTS = [
    'container.0', 'weapon.mainhand', 'weapon.offhand', 'armor.head', 'armor.chest', 'armor.legs', 'armor.feet', 'hotbar.3', 'enderchest.26', 'villager.0',
    'horse.chest.4', 'player.cursor', 'contents', 'inventory.5', 'saddle', 'horse.armor', 'player.crafting.0', 'container.53', 'hotbar.8', 'weapon', 'armor.body',
]
BAD_ITEM_SLOTS = ['container.54', 'nope', 'container.a', 'armor.', 'hotbar.9', 'container.-1', 'container', 'container.0.0', '', 'weapon.', 'armor.nope', 'enderchest.27']
HEX_COLORS = ['ff0000', '#ff0000', '00ff00', 'FFFFFF', '000000', '#abc']
BAD_HEX_COLORS = ['ff00', 'gggggg', '##ff0000', '1234567', '#', '-1']
MESSAGES = ['hello', '@a hello @p', 'hi @e[type=pig] there', 'two  spaces', '@s', 'a @r b @n c', 'x @e[', '@a[x=1]', '@a[limit=0] hi', 'hello world @', '@x hello', '@e[type=nope]']
BAD_MESSAGES = ['@e[', 'hello @x', '@e[type=nope] x']
OBJECTIVE_CRITERIA = ['dummy', 'trigger', 'health', 'minecraft.killed:minecraft.pig', 'minecraft.custom:minecraft.jump', 'teamkill.red', 'killedByTeam.blue',
                      'level', 'food', 'air', 'armor', 'xp', 'deathCount', 'playerKillCount', 'totalKillCount', 'minecraft.mined:minecraft.stone',
                      'minecraft.broken:minecraft.stone', 'minecraft.crafted:minecraft.stone', 'minecraft.used:minecraft.stone',
                      'minecraft.picked_up:minecraft.stone', 'minecraft.dropped:minecraft.stone', 'minecraft.killed_by:minecraft.pig', 'teamkill.', 'killedByTeam.blue']
BAD_OBJECTIVE_CRITERIA = ['bad', 'minecraft.killed:', 'minecraft.killed:minecraft.nope', 'minecraft.custom:nope', 'teamkill.nocolor', 'minecraft.nope:minecraft.pig', '', 'DUMMY', 'minecraft.killed']
OPERATIONS = ['=', '+=', '-=', '*=', '/=', '%=', '<', '>', '><']
BAD_OPERATIONS = ['==', '=+', 'x', '=-', '<>', '+', '<=']
SCOREBOARD_SLOTS = ['sidebar', 'list', 'below_name', 'sidebar.team.red', 'sidebar.team.light_purple', 'sidebar.team.blue']
BAD_SCOREBOARD_SLOTS = ['foo', 'sidebar.team', 'sidebar.team.nocolor', 'sidebar.', 'Sidebar', 'sidebar.team.']
SCORE_HOLDERS = ['Player', '@s', '@e[type=pig]', '*', '#total', '@a[tag=x]', 'fake', 'Steve', '@p', '@e[limit=1]', 'a b']
BAD_SCORE_HOLDERS = ['@x', '@e[', '@e[type=nope]', '@a[limit=-1]']
UUIDS = ['00000000-0000-0000-0000-000000000001', '123e4567-e89b-12d3-a456-426614174000', '1-2-3-4-5', 'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF']
BAD_UUIDS = ['nope', '1234', '00000000-0000-0000-0000', 'g-2-3-4-5', '-', '00000000-0000-0000-0000-0000000000000000000']
SWIZZLES = ['x', 'xy', 'xyz', 'zx', 'yz', 'zyx']
BAD_SWIZZLES = ['xx', 'a', 'xyzx', 'xa', 'X', '']
STYLES = ['{"color":"red"}', '{"bold":true,"italic":false}', '{}', '{color:red}', '{"color":"#ff0000"}', '{"font":"minecraft:uniform"}', '{"underlined":true,"strikethrough":true,"obfuscated":true}']
BAD_STYLES = ['{"color":"x"}', 'red', '{', '"x"', '{"bold":"x"}', '{"color":}', '[]', '{"text":"x"}']
CONTEXT_INTS = ['1', '5', '0', '-1', '{type:"minecraft:uniform",min_inclusive:1,max_inclusive:5}', '{type:"minecraft:constant",value:3}', '{"type":"minecraft:uniform","min":1,"max":5}',
                '{type:"minecraft:binomial",n:5,p:0.5}', '{min:1,max:5}', '[1,2]', '"x"']
BAD_CONTEXT_INTS = ['{', 'x', '{type:"minecraft:nope"}', '{type:"minecraft:uniform"}', '1.5', '{min:5}', '[1', '']
CONTEXT_FLOATS = ['1', '1.5', '0', '-1.5', '{type:"minecraft:uniform",min_inclusive:1,max_inclusive:5}', '{type:"minecraft:constant",value:3}', '{min:1,max:5}']
BAD_CONTEXT_FLOATS = ['{', 'x', '{type:"minecraft:nope"}', '{min:5}', '']
SLOT_SOURCES = ['container.0', 'weapon.mainhand', '{type:"minecraft:slot_range",slots:"container.*"}', '{source:"minecraft:this"}', 'armor.*', 'hotbar.*', 'container.*']
BAD_SLOT_SOURCES = ['{', 'nope', '{type:"minecraft:nope"}', 'container.', '']
ENTITY_ANCHORS = ['feet', 'eyes']
BAD_ENTITY_ANCHORS = ['head', 'Eyes', '']
HEIGHTMAPS = ['world_surface', 'motion_blocking', 'motion_blocking_no_leaves', 'ocean_floor']
BAD_HEIGHTMAPS = ['x', 'surface', 'WORLD_SURFACE']
GAMEMODES = ['survival', 'creative', 'adventure', 'spectator']
BAD_GAMEMODES = ['x', 'Survival', '0', 's', 'survival ']
MIRRORS = ['none', 'left_right', 'front_back']
BAD_MIRRORS = ['x', 'both', 'LEFT_RIGHT']
ROTATIONS = ['none', 'clockwise_90', '180', 'counterclockwise_90']
BAD_ROTATIONS = ['90', 'x', 'clockwise']
SWINGS = ['mainhand', 'offhand']
BAD_SWINGS = ['x', 'main_hand', '']
TEAM_COLORS = ['red', 'blue', 'reset', 'white', 'light_purple', 'dark_aqua', 'black']
BAD_TEAM_COLORS = ['nope', 'Red', 'bold', 'italic']
DIMENSIONS = ['minecraft:overworld', 'overworld', 'the_nether', 'minecraft:the_end']
BAD_DIMENSIONS = ['minecraft:nope', 'nope', 'Bad']
FUNCTIONS = ['foo:bar', '#foo:bar', 'minecraft:x/y', 'foo']
BAD_FUNCTIONS = ['#', 'Foo:bar', '#Foo:bar', 'foo:']
GAME_PROFILES = ['Player', '@a', '@p', 'Steve', '00000000-0000-0000-0000-000000000001', '@s', 'Alex', 'unknown_name']
BAD_GAME_PROFILES = ['@x', '@e[', '@a[limit=-1]']
DIALOGS = ['{type:"minecraft:notice",title:"x"}', 'minecraft:server_links', 'foo:bar']
BAD_DIALOGS = ['{', '{type:"minecraft:nope"}', 'Bad']
BOOLS = ['true', 'false']
BAD_BOOLS = ['maybe', 'TRUE', '1', 'tru', 'True', 'truee']


def sha(text):
    return int(hashlib.sha1(text.encode()).hexdigest()[:8], 16)


def int_samples(props):
    lo = props.get('min', -2147483648) if props else -2147483648
    hi = props.get('max', 2147483647) if props else 2147483647
    cands = [v for v in (0, 1, 5, 10, 100, 1000, -1, -10) if lo <= v <= hi]
    if not cands:
        cands = [lo]
    bad = ['abc', '1.5', '99999999999', '-99999999999', '1e3', '-', '+', '--1', '0x10', '1 ', '2147483648', '-2147483649']
    if lo > -2147483648:
        bad.append(str(lo - 1))
    if hi < 2147483647:
        bad.append(str(hi + 1))
    return [str(c) for c in cands], bad


def double_samples(props, long_like=False):
    lo = props.get('min', -1e308) if props else -1e308
    hi = props.get('max', 1e308) if props else 1e308
    cands = [v for v in ('0', '1', '1.5', '-2.25', '10', '0.5', '100', '.5', '5.') if lo <= float(v) <= hi]
    if not cands:
        cands = [str(lo)]
    bad = ['abc', '1.2.3', 'NaN', '1e', '-', '--1', '1,5', '+', '.', 'infinity']
    if lo > -1e308:
        bad.append(repr(lo - 1))
    if hi < 1e308:
        bad.append(repr(hi + 1))
    return cands, bad


def registry_samples(registries, registry, allow_tag, allow_inline=False):
    entry = registries.get('by_registry', {}).get(registry, {'ids': [], 'tags': []})
    ids = entry['ids']
    valid = []
    if ids:
        valid += [ids[0], ids[len(ids) // 2], ids[-1].split(':', 1)[-1] if ids[-1].startswith('minecraft:') else ids[-1]]
    else:
        valid += ['minecraft:thing']
    if allow_tag and entry['tags']:
        valid.append('#' + entry['tags'][0])
    bad = ['minecraft:nonexistent_thing', 'Bad Id', 'minecraft:']
    if allow_tag:
        bad += ['#minecraft:nonexistent_tag', '#']
    return valid, bad


def pools(parser, props, registries):
    """(valid samples, bad samples) for one argument node."""
    props = props or {}
    if parser == 'brigadier:bool':
        return BOOLS, BAD_BOOLS
    if parser == 'brigadier:integer' or parser == 'brigadier:long':
        return int_samples(props)
    if parser in ('brigadier:double', 'brigadier:float'):
        return double_samples(props)
    if parser == 'brigadier:string':
        kind = props.get('type', 'word')
        if kind == 'greedy':
            return ['hello world', 'a', '@a x', '"q" r', ''], []
        if kind == 'phrase':
            return ['word', '"quoted phrase"', '"esc\\"aped"', '"a\\\\b"', "'single'"], ['"unterminated', '"bad\\escape"', '"x\\']
        return ['word', 'a.b-c_d+e', 'a1'], []
    if parser == 'minecraft:block_pos':
        return BLOCK_POS, BAD_BLOCK_POS
    if parser == 'minecraft:vec3':
        return VEC3, BAD_VEC3
    if parser == 'minecraft:vec2':
        return VEC2, BAD_VEC2
    if parser == 'minecraft:column_pos':
        return COLUMN_POS, BAD_COLUMN_POS
    if parser == 'minecraft:rotation':
        return ROTATION, BAD_ROTATION
    if parser == 'minecraft:entity':
        return SELECTORS, BAD_SELECTORS
    if parser == 'minecraft:score_holder':
        return SCORE_HOLDERS, BAD_SCORE_HOLDERS
    if parser == 'minecraft:game_profile':
        return GAME_PROFILES, BAD_GAME_PROFILES
    if parser == 'minecraft:uuid':
        return UUIDS, BAD_UUIDS
    if parser == 'minecraft:message':
        return MESSAGES, BAD_MESSAGES
    if parser == 'minecraft:component':
        return COMPONENTS, BAD_COMPONENTS
    if parser == 'minecraft:style':
        return STYLES, BAD_STYLES
    if parser == 'minecraft:nbt_compound_tag':
        return NBT_COMPOUNDS, BAD_NBT_COMPOUNDS
    if parser == 'minecraft:nbt_tag':
        return NBT_TAGS, BAD_NBT_TAGS
    if parser == 'minecraft:nbt_path':
        return NBT_PATHS, BAD_NBT_PATHS
    if parser == 'minecraft:objective':
        return ['kills', 'obj', 'health', 'x'], []
    if parser == 'minecraft:objective_criteria':
        return OBJECTIVE_CRITERIA, BAD_OBJECTIVE_CRITERIA
    if parser == 'minecraft:operation':
        return OPERATIONS, BAD_OPERATIONS
    if parser == 'minecraft:scoreboard_slot':
        return SCOREBOARD_SLOTS, BAD_SCOREBOARD_SLOTS
    if parser == 'minecraft:team':
        return ['red', 'blue', 'x'], []
    if parser == 'minecraft:team_color':
        return TEAM_COLORS, BAD_TEAM_COLORS
    if parser == 'minecraft:hex_color':
        return HEX_COLORS, BAD_HEX_COLORS
    if parser == 'minecraft:int_range':
        return INT_RANGES, BAD_INT_RANGES
    if parser == 'minecraft:float_range':
        return FLOAT_RANGES, BAD_FLOAT_RANGES
    if parser == 'minecraft:time':
        return TIMES, BAD_TIMES
    if parser == 'minecraft:resource_location':
        return RESOURCE_LOCATIONS, BAD_RESOURCE_LOCATIONS
    if parser in ('minecraft:resource', 'minecraft:resource_key'):
        return registry_samples(registries, props.get('registry', ''), False)
    if parser in ('minecraft:resource_or_tag', 'minecraft:resource_or_tag_key'):
        return registry_samples(registries, props.get('registry', ''), True)
    if parser == 'minecraft:resource_selector':
        valid, bad = registry_samples(registries, props.get('registry', ''), True)
        return valid + ['*', 'minecraft:*', '#minecraft:*'], bad
    if parser == 'minecraft:dimension':
        return DIMENSIONS, BAD_DIMENSIONS
    if parser == 'minecraft:function':
        return FUNCTIONS, BAD_FUNCTIONS
    if parser == 'minecraft:item_stack':
        return ITEM_STACKS, BAD_ITEM_STACKS
    if parser == 'minecraft:item_predicate':
        return ITEM_PREDICATES, BAD_ITEM_PREDICATES
    if parser == 'minecraft:block_state':
        return BLOCK_STATES, BAD_BLOCK_STATES
    if parser == 'minecraft:block_predicate':
        return BLOCK_PREDICATES, BAD_BLOCK_PREDICATES
    if parser == 'minecraft:particle':
        return PARTICLES, BAD_PARTICLES
    if parser == 'minecraft:item_slot':
        return ITEM_SLOTS, BAD_ITEM_SLOTS
    if parser == 'minecraft:slot_source':
        return SLOT_SOURCES, BAD_SLOT_SOURCES
    if parser == 'minecraft:swizzle':
        return SWIZZLES, BAD_SWIZZLES
    if parser == 'minecraft:entity_anchor':
        return ENTITY_ANCHORS, BAD_ENTITY_ANCHORS
    if parser == 'minecraft:heightmap':
        return HEIGHTMAPS, BAD_HEIGHTMAPS
    if parser == 'minecraft:gamemode':
        return GAMEMODES, BAD_GAMEMODES
    if parser == 'minecraft:template_mirror':
        return MIRRORS, BAD_MIRRORS
    if parser == 'minecraft:template_rotation':
        return ROTATIONS, BAD_ROTATIONS
    if parser == 'minecraft:swing_animation':
        return SWINGS, BAD_SWINGS
    if parser == 'minecraft:context_int_provider':
        return CONTEXT_INTS, BAD_CONTEXT_INTS
    if parser == 'minecraft:context_float_provider':
        return CONTEXT_FLOATS, BAD_CONTEXT_FLOATS
    if parser == 'minecraft:dialog':
        return DIALOGS, BAD_DIALOGS
    if parser in ('minecraft:loot_table', 'minecraft:loot_modifier', 'minecraft:loot_predicate'):
        registry = {'minecraft:loot_table': 'minecraft:loot_table', 'minecraft:loot_modifier': 'minecraft:item_modifier', 'minecraft:loot_predicate': 'minecraft:predicate'}[parser]
        valid, bad = registry_samples(registries, registry, False)
        return valid + ['{}', '{type:"minecraft:chest",pools:[]}', '{"function":"minecraft:set_count","count":1}'], bad + ['{', '{"function":"nope"}']
    if parser == 'minecraft:feature':
        return registry_samples(registries, 'minecraft:worldgen/configured_feature', False)
    return ['0'], ['@@bad@@']


def resolve(tree, path):
    node = tree
    for name in path:
        node = node['children'][name]
    return node


SKIP_ROOTS = {'test', 'jfr', 'perf', 'debug'}


def walk(tree, registries, limit_per_root=0):
    """Yield command token lists: (tokens, kinds) where kinds[i] is None for literals or (valid, bad) pools."""
    out = []

    def rec(node, tokens, kinds, redirects, counter):
        if node.get('executable') and tokens:
            out.append((list(tokens), list(kinds)))
        children = node.get('children', {})
        redirect = node.get('redirect')
        if redirect is not None and redirects < 1:
            target = resolve(tree, redirect)
            rec(target, tokens, kinds, redirects + 1, counter)
        for name, child in children.items():
            if len(tokens) == 0 and name in SKIP_ROOTS:
                continue
            if child['type'] == 'literal':
                tokens.append(name)
                kinds.append(None)
            else:
                valid, bad = pools(child['parser'], child.get('properties'), registries)
                pick = valid[0]
                tokens.append(pick)
                kinds.append((valid, bad, child['parser'], name))
            rec(child, tokens, kinds, redirects, counter)
            tokens.pop()
            kinds.pop()

    rec(tree, [], [], 0, [0])
    return out


def corpus(tree, registries):
    seen = {}

    def add(line):
        if line not in seen:
            seen[line] = True

    paths = walk(tree, registries)
    budget = {}

    def spend(key, limit=2):
        used = budget.get(key, 0)
        if used >= limit:
            return False
        budget[key] = used + 1
        return True

    for tokens, kinds in paths:
        add(' '.join(tokens))
    for tokens, kinds in paths:
        n = len(tokens)
        for i in range(1, n + 1):
            add(' '.join(tokens[:i]))
            add(' '.join(tokens[:i]) + ' ')
        for i, kind in enumerate(kinds):
            if kind is None:
                continue
            valid, bad, parser, name = kind
            head = ' '.join(tokens[:i])
            head = head + ' ' if head else head
            for v in valid:
                if spend((parser, v, 'v')):
                    add(head + v)
                    add(head + v + ' ')
            for b in bad:
                if spend((parser, b, 'b')):
                    add(head + b)
                    add(head + b + ' x')
            value = tokens[i]
            if spend((parser, value, 'cut')):
                for cut in range(1, min(len(value), 6)):
                    add(head + value[:cut])
        if spend((tokens[0], 'extra'), 3):
            add(' '.join(tokens) + ' extra')
            add(' '.join(tokens) + '  ')
    for line in EXTRA:
        add(line)
    for line in [l for l in seen]:
        pass
    return list(seen)


EXTRA = [
    '', ' ', 'nope', 'Help', '/help', 'help ', 'he', 'h', 'g', 'give', 'give ', 'give @s', 'give @s minecraft:stone 0', 'give @s minecraft:stone 100000',
    'tp 1 2 3', 'tp @s 1 2 3', 'tp @s @a', 'tp @e[type=pig] ~ ~ ~', 'tp @s 1 2 3 90 0', 'tp @s ~ ~ ~ facing 0 0 0', 'tp @s ~ ~ ~ facing entity @a eyes',
    'teleport @s ~ ~ ~ facing entity @p feet', 'execute as @a at @s run say hi', 'execute if entity @s run say x', 'execute store result score @s kills run say x',
    'execute positioned ~ ~ ~ rotated ~ ~ run say x', 'execute run', 'execute', 'execute as', 'execute as @a', 'execute foo',
    'execute if score @s kills matches 1..5 run say x', 'execute if block ~ ~ ~ minecraft:stone run say x', 'execute unless dimension minecraft:overworld run say',
    'execute if data entity @s {a:1} run say x', 'execute if predicate minecraft:foo run say x', 'execute summon minecraft:pig run say x',
    'execute on vehicle run say x', 'execute on passengers run say x', 'execute on origin run say x', 'execute on nope run say x',
    'execute align xyz run say x', 'execute align xx run say x', 'execute align q run say x', 'execute anchored eyes run say x',
    'execute in minecraft:the_nether run say x', 'execute in nope run say x', 'execute facing 1 2 3 run say x', 'execute rotated as @s run say x',
    'execute positioned over world_surface run say x', 'execute positioned as @s run say x', 'execute store success storage a:b c int 1.0 run say x',
    'execute store result bossbar a:b value run say x', 'execute store result entity @s Health float 1 run say x',
    'execute store result block 0 0 0 a byte 2 run say x', 'execute if biome ~ ~ ~ minecraft:plains run say x', 'execute if loaded ~ ~ ~ run say x',
    'execute if items entity @s weapon.mainhand minecraft:stone run say x', 'execute if function foo:bar run say x', 'execute if stopwatch foo:bar 1.. run say x',
    'execute if blocks 0 0 0 1 1 1 2 2 2 all run say x', 'execute if blocks 0 0 0 1 1 1 2 2 2 masked run say x', 'execute if blocks 0 0 0 1 1 1 2 2 2 nope run say x',
    'scoreboard objectives add a dummy', 'scoreboard objectives add a dummy "Disp"', 'scoreboard objectives add a dummy {"text":"x"}', 'scoreboard objectives add bad!name dummy',
    'scoreboard objectives add a nope', 'scoreboard players set @s kills 1', 'scoreboard players set @s kills a', 'scoreboard players add Player kills -5',
    'scoreboard players operation @s kills += @a health', 'scoreboard players operation @s kills >< @a health', 'scoreboard objectives setdisplay sidebar kills',
    'scoreboard objectives modify kills numberformat fixed {"text":"x"}', 'scoreboard objectives modify kills numberformat styled {"color":"red"}',
    'scoreboard objectives modify kills numberformat blank', 'scoreboard objectives modify kills rendertype hearts', 'scoreboard objectives modify kills displayautoupdate true',
    'scoreboard players display name @s kills "x"', 'scoreboard players display numberformat @s kills fixed {"text":"x"}', 'scoreboard players enable @s kills',
    'scoreboard players get @s kills', 'scoreboard players list', 'scoreboard players list @s', 'scoreboard players reset *', 'scoreboard players reset @s kills',
    'team add red', 'team add red "Display"', 'team modify red color red', 'team modify red friendlyFire true', 'team modify red prefix {"text":"p"}',
    'team modify red nametagVisibility never', 'team modify red collisionRule pushOtherTeams', 'team modify red deathMessageVisibility hideForOtherTeams',
    'team modify red seeFriendlyInvisibles false', 'team join red @a', 'team join red', 'team leave @a', 'team empty red', 'team remove red', 'team list', 'team list red',
    'data get entity @s', 'data get entity @s Health', 'data get entity @s Health 2', 'data get block 0 0 0 Items[0]', 'data get storage a:b x', 'data merge entity @s {a:1}',
    'data merge block 0 0 0 {a:1}', 'data merge storage a:b {a:1}', 'data modify entity @s a set value 1', 'data modify storage a:b x append value 1',
    'data modify storage a:b x prepend from entity @s Pos', 'data modify storage a:b x insert 0 value 1', 'data modify storage a:b x merge value {a:1}',
    'data modify storage a:b x set string entity @s CustomName 1 2', 'data modify storage a:b x set string entity @s CustomName', 'data modify storage a:b x set from storage a:c y',
    'data remove entity @s a', 'data remove storage a:b x.y[0]', 'data get entity @s a 1.5', 'data modify storage a:b x nope value 1',
    'fill 0 0 0 1 1 1 minecraft:stone', 'fill 0 0 0 1 1 1 stone replace dirt', 'fill 0 0 0 1 1 1 stone outline', 'fill 0 0 0 1 1 1 stone hollow', 'fill 0 0 0 1 1 1 stone destroy',
    'fill 0 0 0 1 1 1 stone keep', 'fill 0 0 0 1 1 1 stone replace', 'fill 0 0 0 1 1 1 stone replace #logs[axis=y]', 'fill 0 0 0 1 1 1 stone nope',
    'fillbiome 0 0 0 1 1 1 minecraft:plains', 'fillbiome 0 0 0 1 1 1 minecraft:plains replace #minecraft:is_ocean', 'clone 0 0 0 1 1 1 2 2 2', 'clone 0 0 0 1 1 1 2 2 2 replace move',
    'clone 0 0 0 1 1 1 2 2 2 masked force', 'clone 0 0 0 1 1 1 2 2 2 filtered stone normal', 'clone from minecraft:overworld 0 0 0 1 1 1 to minecraft:the_nether 2 2 2',
    'setblock 0 0 0 stone', 'setblock 0 0 0 stone destroy', 'setblock 0 0 0 stone keep', 'setblock 0 0 0 stone replace', 'setblock 0 0 0 stone strict', 'setblock ~ ~ ~ oak_stairs[facing=north]{a:1}',
    'summon pig', 'summon pig ~ ~ ~', 'summon pig ~ ~ ~ {NoAI:1b}', 'summon minecraft:nope', 'summon pig ~ ~ ~ {', 'summon minecraft:player', 'summon item ~ ~ ~ {Item:{id:"minecraft:stone",count:1}}',
    'kill', 'kill @e[type=pig]', 'kill Player', 'kill @', 'effect give @s minecraft:speed', 'effect give @s speed 10 1 true', 'effect give @s speed infinite', 'effect give @s speed 0', 'effect give @s speed 1000000 255',
    'effect give @s speed 1000000 256', 'effect give @s nope', 'effect clear @s', 'effect clear @s speed', 'enchant @s sharpness', 'enchant @s minecraft:sharpness 2', 'enchant @s nope',
    'experience add @s 5', 'experience add @s 5 levels', 'experience add @s 5 points', 'experience set @s 5 levels', 'experience query @s levels', 'experience query @s nope', 'xp add @s 1',
    'gamemode creative', 'gamemode creative @s', 'gamemode nope', 'gamemode 1', 'defaultgamemode survival', 'difficulty hard', 'difficulty nope', 'difficulty 1',
    'time set day', 'time set noon', 'time set 1000', 'time set 1d', 'time set 1s', 'time set nope', 'time add 1', 'time add 1d', 'time query daytime', 'time query gametime', 'time query day',
    'time set minecraft:day', 'time of minecraft:overworld set 5', 'time query nope', 'weather clear', 'weather rain 10', 'weather thunder 5t', 'weather clear 1d', 'weather nope',
    'say hello', 'say', 'say @a', 'say @e[', 'me waves', 'msg @a hello', 'tell @a hello', 'w @a hello', 'teammsg hello', 'tm hello', 'tellraw @a {"text":"x"}', 'tellraw @a "x"', 'tellraw @a',
    'title @a title {"text":"x"}', 'title @a subtitle "x"', 'title @a actionbar "x"', 'title @a times 1 2 3', 'title @a times 1s 2s 3s', 'title @a clear', 'title @a reset', 'title @a title', 'title @a nope',
    'list', 'list uuids', 'list nope', 'seed', 'locate structure minecraft:village_plains', 'locate structure #minecraft:village', 'locate biome minecraft:plains', 'locate biome #minecraft:is_ocean',
    'locate poi minecraft:home', 'locate structure nope', 'spawnpoint', 'spawnpoint @s', 'spawnpoint @s 0 0 0', 'spawnpoint @s 0 0 0 90', 'spawnpoint @s ~ ~ ~ ~ ~', 'setworldspawn', 'setworldspawn 0 0 0',
    'setworldspawn 0 0 0 90', 'setworldspawn 0 0 0 90 0', 'clear', 'clear @s', 'clear @s minecraft:stone', 'clear @s stone 0', 'clear @s stone 5', 'clear @s *', 'clear @s #logs', 'clear @s stone[damage=5] 0',
    'gamerule', 'gamerule keepInventory', 'gamerule keep_inventory', 'gamerule keep_inventory true', 'gamerule minecraft:keep_inventory true', 'gamerule max_command_sequence_length 5', 'gamerule nope', 'gamerule keep_inventory 5',
    'worldborder get', 'worldborder set 100', 'worldborder set 100 10', 'worldborder set 100 10s', 'worldborder add 10 5', 'worldborder center 0 0', 'worldborder damage amount 1', 'worldborder damage buffer 1',
    'worldborder warning distance 5', 'worldborder warning time 5', 'worldborder set -1', 'worldborder set 1e10', 'worldborder nope',
    'bossbar add a:b "x"', 'bossbar set a:b color red', 'bossbar set a:b style notched_6', 'bossbar set a:b value 5', 'bossbar set a:b max 5', 'bossbar set a:b visible true', 'bossbar set a:b players @a',
    'bossbar set a:b name "x"', 'bossbar get a:b value', 'bossbar list', 'bossbar remove a:b', 'bossbar set a:b color nope',
    'tag @s add x', 'tag @s remove x', 'tag @s list', 'tag @s add', 'tag @s add "quoted tag"', 'ride @s mount @e[limit=1]', 'ride @s dismount', 'ride @s mount', 'ride @s evict', 'ride @s nope',
    'item replace entity @s weapon.mainhand with stone', 'item replace entity @s weapon.mainhand with stone 5', 'item replace block 0 0 0 container.0 with stone', 'item replace entity @s container.0 from entity @a weapon.mainhand',
    'item replace entity @s container.0 from block 0 0 0 container.1 minecraft:foo', 'item modify entity @s weapon.mainhand minecraft:foo', 'item modify block 0 0 0 container.0 minecraft:foo',
    'loot give @s loot minecraft:chests/simple_dungeon', 'loot give @s kill @e[limit=1]', 'loot give @s mine 0 0 0', 'loot give @s mine 0 0 0 mainhand', 'loot give @s fish minecraft:gameplay/fishing 0 0 0',
    'loot give @s fish minecraft:gameplay/fishing 0 0 0 mainhand', 'loot spawn 0 0 0 loot minecraft:chests/simple_dungeon', 'loot insert 0 0 0 loot minecraft:chests/simple_dungeon', 'loot replace entity @s weapon.mainhand loot minecraft:chests/simple_dungeon',
    'loot replace entity @s weapon.mainhand 2 loot minecraft:chests/simple_dungeon', 'loot replace block 0 0 0 container.0 loot minecraft:chests/simple_dungeon', 'loot give @s nope',
    'particle flame', 'particle flame ~ ~ ~', 'particle flame ~ ~ ~ 1 1 1 0 10', 'particle flame ~ ~ ~ 1 1 1 0 10 force', 'particle flame ~ ~ ~ 1 1 1 0 10 normal @a', 'particle flame ~ ~ ~ 1 1 1 0 0 nope',
    'playsound minecraft:entity.pig.ambient master @s', 'playsound minecraft:entity.pig.ambient master @s ~ ~ ~ 1 1 0', 'playsound entity.pig.ambient nope @s', 'stopsound @s', 'stopsound @s master', 'stopsound @s master minecraft:entity.pig.ambient',
    'stopsound @s * minecraft:entity.pig.ambient', 'attribute @s minecraft:max_health get', 'attribute @s max_health base set 20', 'attribute @s max_health modifier add a:b 1 add_value', 'attribute @s max_health modifier add a:b 1 nope',
    'attribute @s max_health modifier remove a:b', 'attribute @s max_health modifier value get a:b 2', 'attribute @s max_health base get 2', 'attribute @s nope get',
    'damage @s 5', 'damage @s 5 minecraft:generic', 'damage @s 5 minecraft:generic by @a', 'damage @s 5 minecraft:generic by @a from @p', 'damage @s 5 minecraft:generic at 0 0 0', 'damage @s -5', 'damage @s 5 nope',
    'rotate @s 0 0', 'rotate @s ~ ~', 'rotate @s facing 0 0 0', 'rotate @s facing entity @a', 'rotate @s facing entity @a eyes', 'swing @s', 'swing @s mainhand', 'swing @s offhand', 'swing @s nope',
    'spreadplayers 0 0 5 10 false @a', 'spreadplayers 0 0 5 10 under 100 false @a', 'spreadplayers 0 0 5 10 true @a', 'spreadplayers 0 0 0 10 false @a', 'spreadplayers 0 0 5 5 false @a',
    'forceload add 0 0', 'forceload add 0 0 16 16', 'forceload remove all', 'forceload remove 0 0', 'forceload query', 'forceload query 0 0', 'forceload nope', 'recipe give @s *', 'recipe give @s minecraft:stick', 'recipe take @s *',
    'advancement grant @s everything', 'advancement grant @s only minecraft:story/root', 'advancement grant @s only minecraft:story/root criterion', 'advancement grant @s from minecraft:story/root', 'advancement grant @s through minecraft:story/root',
    'advancement grant @s until minecraft:story/root', 'advancement revoke @s everything', 'advancement nope', 'function foo:bar', 'function #foo:bar', 'function foo:bar {a:1}', 'function foo:bar with entity @s a',
    'function foo:bar with storage a:b c', 'function foo:bar with block 0 0 0 a', 'schedule function foo:bar 1t', 'schedule function foo:bar 1d append', 'schedule function foo:bar 1s replace', 'schedule clear foo:bar',
    'schedule function #foo:bar 1t', 'schedule function foo:bar 0t', 'trigger kills', 'trigger kills add 5', 'trigger kills set 5', 'trigger kills nope', 'return 5', 'return fail', 'return run say x', 'return run return 1',
    'random value 1..5', 'random value 1', 'random value 5..1', 'random value ..5', 'random roll 1..5', 'random roll 1..5 foo:bar', 'random reset *', 'random reset foo:bar 5', 'random reset foo:bar 5 true', 'random reset foo:bar 5 true true',
    'random reset * 5 true true 7', 'tick query', 'tick rate 20', 'tick rate 20.5', 'tick rate 0.1', 'tick rate 10000', 'tick rate 1000000', 'tick freeze', 'tick unfreeze', 'tick step', 'tick step 5', 'tick step 5s', 'tick sprint 100', 'tick sprint stop', 'tick nope',
    'reload', 'datapack list', 'datapack list enabled', 'datapack list available', 'datapack enable "vanilla"', 'datapack disable "vanilla"', 'datapack enable "x" first', 'datapack enable "x" last', 'datapack enable "x" before "y"', 'datapack enable "x" after "y"',
    'place feature minecraft:oak', 'place feature minecraft:oak ~ ~ ~', 'place jigsaw minecraft:bastion/starts minecraft:bastion/bridge/start 5', 'place jigsaw a:b c:d 5 ~ ~ ~', 'place structure minecraft:village_plains', 'place structure minecraft:village_plains ~ ~ ~',
    'place template minecraft:igloo/top', 'place template minecraft:igloo/top ~ ~ ~ clockwise_90 front_back 0.5 5', 'place template minecraft:igloo/top ~ ~ ~ nope', 'raid start 5', 'compute',
    'stopwatch create foo:bar', 'stopwatch query foo:bar', 'stopwatch query foo:bar 1.5', 'stopwatch restart foo:bar', 'stopwatch remove foo:bar', 'spectate', 'spectate @e[limit=1]', 'spectate @e[limit=1] @s', 'spectate @e[limit=1] nope',
    'waypoint list', 'waypoint modify @s color red', 'waypoint modify @s color hex ff0000', 'waypoint modify @s style default', 'waypoint modify @s style reset', 'waypoint modify @s color reset', 'waypoint modify @s style nope',
    'dialog show @s foo:bar', 'dialog clear @s', 'dialog show @s {type:"minecraft:notice",title:"x"}', 'publish', 'publish true', 'publish true survival 25565', 'publish false creative', 'publish nope', 'unpublish',
    'version', 'help execute', 'help execute run', 'help nope', 'help give', 'help tp', 'kick @s', 'kick @s "reason"', 'fetchprofile name Notch', 'fetchprofile id 00000000-0000-0000-0000-000000000001',
    'posteffect @s minecraft:x', 'posteffect', 'ban Player', 'op Player', 'whitelist add Player', 'stop', 'save-all', 'debug start', 'debug stop', 'debug report', 'debug function foo:bar',
    'time set 1', 'time set  1', ' time set 1', 'time\tset 1', 'say \u00e9\u00e9', 'say \U0001f600', 'tp \u00e9 1 2 3', 'give \U0001f600 stone', 'execute as @e[name="\u00e9"] run say \u00e9',
]
