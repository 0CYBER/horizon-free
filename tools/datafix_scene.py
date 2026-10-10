"""Scenes for tools/datafix_worlds.py: block entities, entities and items in the save
formats of older game versions, placed by console commands of that version.

Three syntax eras matter for the commands:
  A  before 1.20.5 (DataVersion < 3818): item `tag` NBT, `Count`, JSON text strings
  B  1.20.5 to 1.21.4 (< 4325): item components, JSON text strings
  C  1.21.5 and later: item components, SNBT text components, `equipment`

Every entry is (minimum DataVersion, command template). `{x} {y} {z}` are filled
with a free position; entities get a fixed UUID so outputs can be matched.
"""

A, B, C = 'A', 'B', 'C'


def era(version):
    return A if version < 3818 else B if version < 4325 else C


def text(e, s):
    """A text component value for NBT: JSON string (A/B) or SNBT compound (C)."""
    if e == C:
        return '{text:"%s"}' % s
    return "'{\"text\":\"%s\"}'" % s


# ------------------------------------------------------------------ items, era A
ITEMS_A = [
    '{id:"minecraft:diamond_sword",Count:1b,tag:{Damage:5,RepairCost:3,Unbreakable:1b,HideFlags:255,'
    'Enchantments:[{id:"minecraft:sharpness",lvl:3s},{id:"minecraft:sweeping",lvl:2s}],'
    'display:{Name:\'{"text":"Blade","color":"red","italic":false}\',Lore:[\'{"text":"line one"}\',\'"plain"\']},'
    'AttributeModifiers:[{AttributeName:"generic.attack_damage",Name:"x",Amount:2.5d,Operation:0,'
    'UUID:[I;1,2,3,4],Slot:"mainhand"}],CustomModelData:7}}',
    '{id:"minecraft:enchanted_book",Count:1b,tag:{StoredEnchantments:[{id:"minecraft:mending",lvl:1s}]}}',
    '{id:"minecraft:written_book",Count:1b,tag:{title:"Title",author:"Me",generation:1,resolved:1b,'
    'pages:[\'{"text":"page one","bold":true}\',\'{"text":"two","clickEvent":{"action":"change_page","value":"2"},'
    '"hoverEvent":{"action":"show_text","value":"hi"}}\'],filtered_pages:{"1":\'"f"\'}}}',
    '{id:"minecraft:writable_book",Count:1b,tag:{pages:["draft one","draft two"]}}',
    '{id:"minecraft:potion",Count:1b,tag:{Potion:"minecraft:strong_healing",CustomPotionColor:255}}',
    '{id:"minecraft:splash_potion",Count:1b,tag:{CustomPotionEffects:[{Id:1b,Amplifier:1b,Duration:200}]}}',
    '{id:"minecraft:tipped_arrow",Count:5b,tag:{Potion:"minecraft:poison"}}',
    '{id:"minecraft:suspicious_stew",Count:1b,tag:{Effects:[{EffectId:3,EffectDuration:160}]}}',
    '{id:"minecraft:firework_rocket",Count:3b,tag:{Fireworks:{Flight:2b,Explosions:[{Type:1b,Colors:[I;255],'
    'FadeColors:[I;16711680],Trail:1b,Flicker:0b}]}}}',
    '{id:"minecraft:firework_star",Count:1b,tag:{Explosion:{Type:3b,Colors:[I;65280]}}}',
    '{id:"minecraft:player_head",Count:1b,tag:{SkullOwner:{Name:"Notch",Id:[I;1,2,3,4],'
    'Properties:{textures:[{Value:"abc",Signature:"sig"}]}}}}',
    '{id:"minecraft:player_head",Count:1b,tag:{SkullOwner:"jeb_"}}',
    '{id:"minecraft:filled_map",Count:1b,tag:{map:3,display:{MapColor:255},Decorations:[{id:"x",type:9b,x:1.0d,z:2.0d,rot:180.0d}]}}',
    '{id:"minecraft:compass",Count:1b,tag:{LodestonePos:{X:1,Y:2,Z:3},LodestoneDimension:"minecraft:overworld",LodestoneTracked:0b}}',
    '{id:"minecraft:crossbow",Count:1b,tag:{Charged:1b,ChargedProjectiles:[{id:"minecraft:arrow",Count:1b}]}}',
    '{id:"minecraft:shield",Count:1b,tag:{BlockEntityTag:{Base:4,Patterns:[{Pattern:"cre",Color:15},{Pattern:"bs",Color:1}]}}}',
    '{id:"minecraft:white_banner",Count:1b,tag:{BlockEntityTag:{Patterns:[{Pattern:"mr",Color:9}]},'
    'display:{Name:\'{"translate":"block.minecraft.ominous_banner","color":"gold"}\'}}}',
    '{id:"minecraft:red_shulker_box",Count:1b,tag:{BlockEntityTag:{Items:[{Slot:3b,id:"minecraft:stone",Count:9b},'
    '{Slot:0b,id:"minecraft:golden_apple",Count:2b,tag:{display:{Name:\'"Gold"\'}}}],Lock:"key",'
    'CustomName:\'{"text":"Box"}\'}}}',
    '{id:"minecraft:chest",Count:1b,tag:{BlockEntityTag:{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:5L}}}',
    '{id:"minecraft:pig_spawn_egg",Count:1b,tag:{EntityTag:{CustomName:\'"Babe"\',Age:-24000}}}',
    '{id:"minecraft:armor_stand",Count:1b,tag:{EntityTag:{Small:1b}}}',
    '{id:"minecraft:leather_chestplate",Count:1b,tag:{display:{color:16711680},'
    'Trim:{material:"minecraft:gold",pattern:"minecraft:coast"}}}',
    '{id:"minecraft:goat_horn",Count:1b,tag:{instrument:"minecraft:sing_goat_horn"}}',
    '{id:"minecraft:axolotl_bucket",Count:1b,tag:{Variant:2,Age:-100,Health:7.0f,NoAI:1b}}',
    '{id:"minecraft:tropical_fish_bucket",Count:1b,tag:{BucketVariantTag:65536}}',
    '{id:"minecraft:knowledge_book",Count:1b,tag:{Recipes:["minecraft:stone"]}}',
    '{id:"minecraft:debug_stick",Count:1b,tag:{DebugProperty:{"minecraft:oak_stairs":"facing"}}}',
    '{id:"minecraft:diamond_pickaxe",Count:1b,tag:{CanDestroy:["minecraft:stone","#minecraft:logs",'
    '"minecraft:oak_stairs[facing=east]"],HideFlags:8}}',
    '{id:"minecraft:stone",Count:4b,tag:{CanPlaceOn:["minecraft:dirt"],BlockStateTag:{}}}',
    '{id:"minecraft:oak_slab",Count:1b,tag:{BlockStateTag:{type:"top",waterlogged:1b}}}',
    '{id:"minecraft:grass",Count:2b}',
    '{id:"minecraft:scute",Count:2b}',
    '{id:"minecraft:chain",Count:1b}',
    '{id:"minecraft:decorated_pot",Count:1b,tag:{BlockEntityTag:{sherds:["minecraft:brick","minecraft:angler_pottery_sherd",'
    '"minecraft:brick","minecraft:arms_up_pottery_sherd"]}}}',
    '{id:"minecraft:beehive",Count:1b,tag:{BlockEntityTag:{Bees:[{EntityData:{id:"minecraft:bee"},TicksInHive:3,'
    'MinOccupationTicks:600}]}}}',
    '{id:"minecraft:cooked_beef",Count:64b,tag:{custom:"data",nested:{a:1}}}',
    '{id:"minecraft:iron_sword",Count:1b,tag:{Enchantments:[]}}',
    '{id:"minecraft:bow",Count:1b,tag:{Enchantments:[{id:"minecraft:power",lvl:0s}],HideFlags:1}}',
]

# ------------------------------------------------------------------ items, era B/C
ITEMS_B = [
    '{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:repair_cost":3,'
    '"minecraft:unbreakable":{show_in_tooltip:false},"minecraft:enchantments":{levels:{"minecraft:sharpness":3}},'
    '"minecraft:custom_name":\'{"text":"Blade","color":"red"}\',"minecraft:lore":[\'"plain"\'],'
    '"minecraft:attribute_modifiers":{modifiers:[{type:"minecraft:generic.attack_damage",id:"minecraft:x",'
    'amount:2.5d,operation:"add_value",slot:"mainhand"}]}}}',
    '{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"T"},author:"Me",'
    'pages:[{raw:\'{"text":"p1","clickEvent":{"action":"run_command","value":"/say hi"}}\'}]}}}',
    '{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:healing"}}}',
    '{id:"minecraft:white_banner",count:1,components:{"minecraft:banner_patterns":[{pattern:"minecraft:creeper",color:"black"}],'
    '"minecraft:item_name":\'{"translate":"block.minecraft.ominous_banner"}\',"minecraft:rarity":"uncommon"}}',
    '{id:"minecraft:red_shulker_box",count:1,components:{"minecraft:container":[{slot:0,item:{id:"minecraft:stone",count:3}}],'
    '"minecraft:lock":"key"}}',
    '{id:"minecraft:apple",count:2,components:{"minecraft:food":{nutrition:4,saturation:2.4f,can_always_eat:true,'
    'eat_seconds:1.0f}}}',
    '{id:"minecraft:netherite_sword",count:1,components:{"minecraft:fire_resistant":{}}}',
    '{id:"minecraft:stick",count:1,components:{"minecraft:custom_model_data":5}}',
    '{id:"minecraft:pig_spawn_egg",count:1,components:{"minecraft:entity_data":{id:"minecraft:pig",Age:-200}}}',
    '{id:"minecraft:diamond_helmet",count:1,components:{"minecraft:hide_additional_tooltip":{},'
    '"minecraft:trim":{material:"minecraft:gold",pattern:"minecraft:coast",show_in_tooltip:false}}}',
    '{id:"minecraft:decorated_pot",count:1,components:{"minecraft:pot_decorations":["minecraft:brick",'
    '"minecraft:angler_pottery_sherd"]}}',
    '{id:"minecraft:chain",count:1}',
]
ITEMS_C = [
    '{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,'
    '"minecraft:custom_name":{text:"Blade",color:"red"},"minecraft:tooltip_display":{hidden_components:["minecraft:damage"]}}}',
    '{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"T"},author:"Me",'
    'pages:[{raw:{text:"p1",click_event:{action:"change_page",page:2}}}]}}}',
    '{id:"minecraft:decorated_pot",count:1,components:{"minecraft:pot_decorations":["minecraft:brick",'
    '"minecraft:angler_pottery_sherd"]}}',
    '{id:"minecraft:filled_map",count:1,components:{"minecraft:map_id":2,"minecraft:map_color":255}}',
    '{id:"minecraft:chain",count:1}',
    '{id:"minecraft:name_tag",count:1,components:{"minecraft:custom_name":"Bob"}}',
]


def items(e):
    return ITEMS_A if e == A else ITEMS_B if e == B else ITEMS_C


def slotted(stacks):
    out = []
    for i, s in enumerate(stacks[:27]):
        out.append(s[:1] + f'Slot:{i}b,' + s[1:])
    return '[' + ','.join(out) + ']'


def chests(e):
    """Chests holding every sample item (27 per chest)."""
    st = items(e)
    return [st[i:i + 27] for i in range(0, len(st), 27)]


# ------------------------------------------------------------------ block entities
def block_entities(e, version):
    name = lambda s: text(e, s)
    out = []
    for group in chests(e):
        out.append(('chest', f'{{Items:{slotted(group)},CustomName:{name("Stash")}}}'))
    sign_a = ('{front_text:{messages:[\'{"text":"one"}\',\'"two"\',\'""\',\'""\'],color:"red",has_glowing_text:1b},'
              'back_text:{messages:[\'"b"\',\'""\',\'""\',\'""\'],filtered_messages:[\'"x"\',\'""\',\'""\',\'""\']},'
              'is_waxed:1b}')
    sign_c = ('{front_text:{messages:[{text:"one"},"two","",""],color:"red",has_glowing_text:1b},'
              'back_text:{messages:["b","","",""]},is_waxed:1b}')
    sign = sign_c if e == C else sign_a
    out += [('oak_sign', sign), ('oak_hanging_sign', sign)]
    if e == A:
        out += [
            ('white_banner', '{Patterns:[{Pattern:"cre",Color:15}],CustomName:\'{"translate":"block.minecraft.ominous_banner"}\'}'),
            ('player_head', '{SkullOwner:{Name:"Notch",Id:[I;1,2,3,4]}}'),
            ('decorated_pot', '{sherds:["minecraft:brick","minecraft:skull_pottery_sherd","minecraft:brick","minecraft:brick"]}'),
            ('beehive', '{Bees:[{EntityData:{id:"minecraft:bee",CustomName:\'"Bz"\'},TicksInHive:2,MinOccupationTicks:400}],'
                        'FlowerPos:{X:1,Y:2,Z:3}}'),
            ('furnace', '{BurnTime:100s,CookTime:20s,CookTimeTotal:200s,Items:[{Slot:0b,id:"minecraft:iron_ore",Count:3b}],'
                        'RecipesUsed:{"minecraft:iron_ingot":2}}'),
            ('jukebox', '{RecordItem:{id:"minecraft:music_disc_cat",Count:1b},RecordStartTick:100L,TickCount:250L,IsPlaying:1b}'),
            ('suspicious_sand', '{item:{id:"minecraft:emerald",Count:1b}}'),
            ('lectern', '{Book:{id:"minecraft:written_book",Count:1b,tag:{title:"L",author:"A",pages:[\'"x"\']}},Page:0}'),
            ('spawner', '{SpawnData:{entity:{id:"minecraft:zombie",ArmorItems:[{},{},{},{id:"minecraft:iron_helmet",Count:1b}]}},'
                        'SpawnPotentials:[{weight:1,data:{entity:{id:"minecraft:skeleton"}}}]}'),
            ('command_block', '{Command:"say hi",CustomName:\'"Cmd"\',LastOutput:\'{"text":"out"}\'}'),
            ('end_gateway', '{ExitPortal:{X:1,Y:70,Z:2},ExactTeleport:1b,Age:5L}'),
            ('chiseled_bookshelf', '{Items:[{Slot:1b,id:"minecraft:book",Count:1b}],last_interacted_slot:1}'),
            ('campfire', '{Items:[{Slot:0b,id:"minecraft:beef",Count:1b}],CookingTimes:[I;5,0,0,0],CookingTotalTimes:[I;600,0,0,0]}'),
            ('hopper', '{Items:[{Slot:0b,id:"minecraft:stone",Count:1b}],Lock:"key"}'),
            ('structure_block', '{name:"x:y",posX:1,posY:2,posZ:3,sizeX:4,sizeY:5,sizeZ:6,mode:"SAVE"}'),
            ('skeleton_skull', '{}'),
        ]
        if version >= 3682:
            out.append(('crafter', '{Items:[{Slot:0b,id:"minecraft:stone",Count:1b}],triggered:1,crafting_ticks_remaining:2}'))
    else:
        out += [
            ('white_banner', '{patterns:[{pattern:"minecraft:creeper",color:"black"}],CustomName:%s}' % name("Flag")),
            ('player_head', '{profile:{name:"Notch"}}'),
            ('decorated_pot', '{sherds:["minecraft:brick","minecraft:skull_pottery_sherd","minecraft:brick","minecraft:brick"]}'),
            ('furnace', '{BurnTime:100s,CookTime:20s,CookTimeTotal:200s,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}'
             if version < 4181 else '{lit_time_remaining:100s,cooking_time_spent:20s,cooking_total_time:200s}'),
            ('jukebox', '{RecordItem:{id:"minecraft:music_disc_cat",count:1},ticks_since_song_started:5L}'),
            ('trial_spawner', '{required_player_range:10}'),
            ('vault', '{config:{key_item:{id:"minecraft:trial_key",count:1}}}'),
            ('crafter', '{Items:[{Slot:0b,id:"minecraft:stone",count:1}],triggered:1}'),
            ('spawner', '{SpawnData:{entity:{id:"minecraft:zombie"}}}'),
            ('hopper', '{Items:[{Slot:0b,id:"minecraft:stone",count:1}],lock:{items:"minecraft:stick"}}'
             if version >= 4068 else '{Items:[{Slot:0b,id:"minecraft:stone",count:1}],Lock:"key"}'),
        ]
        if version >= 4071:
            out.append(('creaking_heart', ''))
        if version >= 4533:
            out.append(('oak_shelf', '{Items:[{Slot:0b,id:"minecraft:apple",count:1}]}'))
        if version >= 4532:
            out.append(('copper_golem_statue', ''))
    return out


# ------------------------------------------------------------------ entities
def entities(e, version):
    """(id, SNBT fields) for summon; item fields in the era's form."""
    if e == A:
        sword = '{id:"minecraft:iron_sword",Count:1b,tag:{display:{Name:\'"S"\'}}}'
        helmet = '{id:"minecraft:iron_helmet",Count:1b}'
        out = [
            ('zombie', f'ArmorItems:[{{}},{{}},{{}},{helmet}],HandItems:[{sword},{{}}],ArmorDropChances:[0.1f,0.1f,0.1f,2.0f],'
                       'HandDropChances:[1.0f,0.085f],CustomName:\'"Zed"\''),
            ('villager', 'VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"},Xp:3,'
                         'Offers:{Recipes:[{buy:{id:"minecraft:wheat",Count:20b},buyB:{id:"minecraft:air",Count:1b},'
                         'sell:{id:"minecraft:emerald",Count:1b},maxUses:16,uses:1}]},Inventory:[{id:"minecraft:bread",Count:3b}]'),
            ('horse', 'Tame:1b,SaddleItem:{id:"minecraft:saddle",Count:1b},ArmorItem:{id:"minecraft:iron_horse_armor",Count:1b},'
                      'Attributes:[{Name:"minecraft:horse.jump_strength",Base:0.7d},{Name:"minecraft:generic.max_health",Base:20.0d,'
                      'Modifiers:[{Name:"m",Amount:1.0d,Operation:0,UUID:[I;5,6,7,8]}]}]'),
            ('llama', 'ChestedHorse:1b,Items:[{Slot:2b,id:"minecraft:apple",Count:1b}],DecorItem:{id:"minecraft:red_carpet",Count:1b}'),
            ('donkey', 'ChestedHorse:1b,Items:[{Slot:2b,id:"minecraft:apple",Count:1b}],SaddleItem:{id:"minecraft:saddle",Count:1b}'),
            ('wolf', 'Health:15.0f,Owner:[I;1,2,3,4],CollarColor:3b'),
            ('salmon', ''),
            ('armor_stand', f'ArmorItems:[{{}},{{}},{{}},{helmet}],HandItems:[{sword},{{}}],Small:1b'),
            ('item_frame', f'Item:{sword},Facing:3b'),
            ('item', 'Item:{id:"minecraft:written_book",Count:1b,tag:{pages:[\'"p"\'],title:"t",author:"a"}},PickupDelay:10s'),
            ('arrow', 'inGround:1b,Potion:"minecraft:poison",CustomPotionEffects:[{Id:19b,Duration:100}]'),
            ('trident', 'Trident:{id:"minecraft:trident",Count:1b},inGround:1b'),
            ('area_effect_cloud', 'Potion:"minecraft:healing",Particle:"entity_effect",Radius:2.0f,Duration:600'),
            ('potion', 'Item:{id:"minecraft:splash_potion",Count:1b,tag:{Potion:"minecraft:swiftness"}}'),
            ('falling_block', 'BlockState:{Name:"minecraft:sand"},Time:-1000,NoGravity:1b'),
            ('tnt', 'Fuse:80s'),
            ('tnt_minecart', 'TNTFuse:40'),
            ('chest_minecart', 'Items:[{Slot:0b,id:"minecraft:apple",Count:1b}]'),
            ('boat', 'Type:"spruce"'),
            ('chest_boat', 'Type:"birch",Items:[{Slot:0b,id:"minecraft:apple",Count:1b}]'),
            ('text_display', 'text:\'{"text":"Hi","bold":true}\''),
            ('item_display', 'item:{id:"minecraft:stone",Count:1b}'),
            ('pig', 'Saddle:1b'),
            ('cat', 'variant:"minecraft:black"'),
            ('fox', 'Type:"red"'),
            ('zombie_villager', 'VillagerData:{profession:"minecraft:none",level:1,type:"minecraft:plains"}'),
            ('wandering_trader', ''),
            ('piglin', 'Inventory:[{id:"minecraft:gold_ingot",Count:2b}]'),
            ('firework_rocket', 'FireworksItem:{id:"minecraft:firework_rocket",Count:1b,tag:{Fireworks:{Flight:1b}}}'),
            ('allay', 'Inventory:[{id:"minecraft:diamond",Count:1b}]'),
        ]
        return out
    sword = '{id:"minecraft:iron_sword",count:1}'
    helmet = '{id:"minecraft:iron_helmet",count:1}'
    name = lambda s: text(e, s)
    if e == B:
        out = [
            ('zombie', f'ArmorItems:[{{}},{{}},{{}},{helmet}],HandItems:[{sword},{{}}],ArmorDropChances:[0.1f,0.1f,0.1f,2.0f],'
                       f'CustomName:{name("Zed")}'),
            ('horse', 'Tame:1b,SaddleItem:{id:"minecraft:saddle",count:1},body_armor_item:{id:"minecraft:iron_horse_armor",count:1}'),
            ('llama', 'ChestedHorse:1b,Items:[{Slot:2b,id:"minecraft:apple",count:1}],body_armor_item:{id:"minecraft:red_carpet",count:1}'),
            ('area_effect_cloud', 'potion_contents:{potion:"minecraft:healing"},Particle:{type:"minecraft:entity_effect",color:255},Duration:600'),
            ('potion', 'Item:{id:"minecraft:lingering_potion",count:1}'),
            ('tnt_minecart', 'TNTFuse:40' if version < 4173 else 'fuse:40'),
            ('salmon', ''),
            ('item_frame', f'Item:{sword},Facing:3b'),
            ('arrow', 'inGround:1b,item:{id:"minecraft:arrow",count:1}'),
            ('text_display', f'text:{name("Hi")}'),
            ('villager', 'VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"}'),
            ('pig', 'Saddle:1b'),
            ('falling_block', 'BlockState:{Name:"minecraft:sand"},Time:-1000,NoGravity:1b'),
        ]
        if version >= 4067:
            out.append(('pale_oak_chest_boat' if version >= 4070 else 'oak_chest_boat', 'Items:[{Slot:0b,id:"minecraft:apple",count:1}]'))
        else:
            out.append(('chest_boat', 'Type:"birch",Items:[{Slot:0b,id:"minecraft:apple",count:1}]'))
        return out
    out = [
        ('zombie', f'equipment:{{head:{helmet},mainhand:{sword}}},drop_chances:{{head:2.0f}},CustomName:{name("Zed")}'),
        ('horse', 'Tame:1b,equipment:{saddle:{id:"minecraft:saddle",count:1},body:{id:"minecraft:iron_horse_armor",count:1}}'),
        ('area_effect_cloud', 'potion_contents:{potion:"minecraft:healing"},Duration:600'),
        ('splash_potion' if version >= 4306 else 'potion', 'Item:{id:"minecraft:splash_potion",count:1}'),
        ('tnt_minecart', 'fuse:40'),
        ('text_display', f'text:{name("Hi")}'),
        ('villager', 'VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"}'),
        ('falling_block', 'BlockState:{Name:"minecraft:sand"},Time:-1000,NoGravity:1b'),
        ('trident', 'item:{id:"minecraft:trident",count:1},inGround:1b'),
    ]
    if version >= 4531:
        out.append(('copper_golem', ''))
    if version >= 4543:
        out.append(('mannequin', ''))
    return out


def player_commands(e, name):
    """Give the joined bot a varied inventory and ender chest."""
    out = []
    for i, stack in enumerate(items(e)[:30]):
        out.append(f'item replace entity {name} container.{i} with {give_form(e, stack)}')
    out.append(f'item replace entity {name} enderchest.0 with {give_form(e, items(e)[0])}')
    out.append(f'item replace entity {name} armor.head with minecraft:iron_helmet')
    out.append(f'xp set {name} 5 levels')
    return out


def give_form(e, stack):
    """`item replace ... with` argument from a stack SNBT: id plus tag/components."""
    import re
    m = re.match(r'\{id:"([^"]+)",(?:Count:(\d+)b|count:(\d+))(?:,(tag|components):(.*))?\}$', stack, re.S)
    item, count = m.group(1), m.group(2) or m.group(3)
    if m.group(4) == 'tag':
        return f'{item}{m.group(5)} {count}'
    if m.group(4) == 'components':
        body = m.group(5)[1:-1]
        parts = split_top(body)
        comps = []
        for p in parts:
            if p.startswith('"'):
                end = p.index('"', 1)
                key, value = p[1:end], p[end + 1:].lstrip()[1:]
            else:
                key, value = p.split(':', 1)
            comps.append(f'{key}={value.strip()}')
        comps = ','.join(comps)
        return f'{item}[{comps}] {count}'
    return f'{item} {count}'


def split_top(s):
    """Split SNBT compound contents at top-level commas."""
    out, depth, cur, q = [], 0, '', None
    for ch in s:
        if q:
            cur += ch
            if ch == q:
                q = None
            continue
        if ch in '"\'':
            q = ch
        elif ch in '{[':
            depth += 1
        elif ch in '}]':
            depth -= 1
        elif ch == ',' and depth == 0:
            out.append(cur)
            cur = ''
            continue
        cur += ch
    if cur:
        out.append(cur)
    return out
