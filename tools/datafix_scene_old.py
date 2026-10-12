"""Scene additions for worlds saved by 1.13.2 to 1.16.5 (DataVersion 1631 to 2586): the entities, block
entities and items whose save format those versions changed, in the syntax and format of the version
that writes them (numeric UUIDMost/UUIDLeast, string owner UUIDs, Profession/Career villagers, ...).

Every entry is (minimum DataVersion, maximum DataVersion exclusive, value); datafix_scene.py adds the
ones valid for the world's version to its general scene.
"""

U1 = '11111111-2222-3333-4444-555555555555'
U2 = '66666666-7777-8888-9999-aaaaaaaaaaaa'
MODIFIERS = ('[{Name:"horse.jumpStrength",Base:0.7d},{Name:"generic.maxHealth",Base:20.0d,'
             'Modifiers:[{Name:"m",Amount:1.0d,Operation:0,UUIDMost:5L,UUIDLeast:6L}]}]')
TRADES = ('{buy:{id:"minecraft:wheat",Count:20b},sell:{id:"minecraft:emerald",Count:1b},maxUses:16,uses:1,rewardExp:1b},'
          '{buy:{id:"minecraft:paper",Count:24b},sell:{id:"minecraft:emerald",Count:1b},maxUses:16,uses:0},'
          '{buy:{id:"minecraft:book",Count:4b},sell:{id:"minecraft:emerald",Count:1b},maxUses:12,uses:0},'
          '{buy:{id:"minecraft:ink_sac",Count:5b},sell:{id:"minecraft:emerald",Count:1b},maxUses:12,uses:0}')

# (since, until, entity, fields)
ENTITIES = [
    # 1.13.2: professions are numbers
    (1631, 1900, 'villager', 'Profession:1,Career:2,CareerLevel:2,Offers:{Recipes:[%s]},Inventory:[{id:"minecraft:bread",Count:3b}]' % TRADES),
    (1631, 1900, 'villager', 'Profession:3,Career:3,CareerLevel:1'),
    (1631, 1900, 'zombie_villager', 'Profession:4,Career:2'),
    (1631, 1900, 'ocelot', 'CatType:2,OwnerUUID:"%s"' % U1),
    (1631, 1900, 'ocelot', 'CatType:0'),
    (1631, 1900, 'ocelot', 'CatType:0,OwnerUUID:"%s"' % U2),
    # 1.14: VillagerData and Xp; some without them
    (1900, 2100, 'villager', 'VillagerData:{profession:"minecraft:farmer",level:1,type:"minecraft:plains"},'
                             'Offers:{Recipes:[%s]},Gossips:[{Type:"golem",Value:5},{Type:"major_positive",Value:2}]' % TRADES),
    (1900, 2100, 'villager', 'VillagerData:{profession:"minecraft:armorer",level:3,type:"minecraft:desert"},Xp:70'),
    (1900, 2100, 'zombie_villager', 'VillagerData:{profession:"minecraft:cleric",level:2,type:"minecraft:plains"}'),
    (1900, 2100, 'cat', 'CatType:9'),
    (1900, 2100, 'cat', 'CatType:3,CollarColor:5b'),
    (1900, 2100, 'pillager', 'Inventory:[{id:"minecraft:arrow",Count:2b}]'),
    (1900, 2100, 'fox', 'Type:"red",TrustedUUIDs:[{M:3L,L:4L}]'),
    (1631, 2514, 'wolf', 'Owner:"%s",OwnerUUID:"%s",CollarColor:3b,Health:15.0f' % ('Steve', U1)),
    (1631, 2514, 'horse', 'Tame:1b,OwnerUUID:"%s",SaddleItem:{id:"minecraft:saddle",Count:1b},'
                          'ArmorItem:{id:"minecraft:iron_horse_armor",Count:1b},Attributes:%s' % (U2, MODIFIERS)),
    (1631, 2514, 'armor_stand', 'Attributes:%s' % MODIFIERS),
    (1631, 2514, 'cow', 'Leash:{UUIDMost:7L,UUIDLeast:8L}'),
    (1631, 2509, 'zombie_pigman', 'Anger:100s,HurtBy:"%s"' % U1),
    (1631, 2514, 'item', 'Item:{id:"minecraft:diamond",Count:3b},Owner:{M:9L,L:10L},Thrower:{M:11L,L:12L}'),
    (1631, 2514, 'area_effect_cloud', 'OwnerUUIDMost:13L,OwnerUUIDLeast:14L,Particle:"entity_effect",Radius:2.0f,Duration:600'),
    (1631, 2514, 'arrow', 'OwnerUUIDMost:15L,OwnerUUIDLeast:16L,inGround:1b'),
    (1631, 2514, 'trident', 'OwnerUUIDMost:17L,OwnerUUIDLeast:18L,Trident:{id:"minecraft:trident",Count:1b}'),
    (1631, 2514, 'potion', 'Potion:{id:"minecraft:splash_potion",Count:1b,tag:{Potion:"minecraft:swiftness"}},owner:{M:19L,L:20L}'),
    (1631, 2514, 'snowball', 'owner:{M:21L,L:22L}'),
    (1631, 2514, 'egg', 'owner:{M:23L,L:24L}'),
    (1631, 2514, 'ender_pearl', 'owner:{M:25L,L:26L}'),
    (1631, 2514, 'experience_bottle', 'owner:{M:27L,L:28L}'),
    (1631, 2514, 'llama_spit', 'Owner:{OwnerUUIDMost:29L,OwnerUUIDLeast:30L}'),
    (1631, 2514, 'shulker_bullet', 'Owner:{M:31L,L:32L},Target:{M:33L,L:34L}'),
    (1631, 2514, 'evoker_fangs', 'OwnerUUIDMost:35L,OwnerUUIDLeast:36L'),
    (1631, 2514, 'shulker', 'Rotation:[40.0f,0.0f],Peek:0b'),
    (1631, 2514, 'bat', ''),
    (2100, 2514, 'bee', 'HurtBy:"%s",HasNectar:1b,Anger:50' % U1),
    (2100, 2514, 'villager', 'VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"},Xp:20,'
                             'Brain:{memories:{"minecraft:job_site":{pos:[I;1,2,3],dimension:"minecraft:overworld"}}}'),
]

# (since, until, block, nbt)
BLOCK_ENTITIES = [
    (1631, 2514, 'white_banner', '{CustomName:\'{"translate":"block.minecraft.illager_banner","color":"gold"}\','
                                 'Patterns:[{Pattern:"cre",Color:15}]}'),
    (1631, 2514, 'chest', '{LootTable:"minecraft:chests/village_blacksmith"}'),
    (1631, 2514, 'furnace', '{BurnTime:100s,CookTime:20s,CookTimeTotal:200s,RecipesUsedSize:2,RecipeLocation0:"minecraft:iron_ingot",'
                            'RecipeAmount0:3,RecipeLocation1:"minecraft:glass",RecipeAmount1:1}'),
    (1631, 2514, 'blast_furnace', '{RecipesUsedSize:1,RecipeLocation0:"minecraft:iron_ingot",RecipeAmount0:2}') if False else None,
    (1631, 2514, 'player_head', '{Owner:{Id:"%s",Name:"Notch"}}' % U1),
    (1631, 2514, 'conduit', '{target_uuid:{M:5L,L:6L},Target:{M:5L,L:6L}}'),
    (1631, 2514, 'oak_sign', '{Text1:\'{"text":"one"}\',Text2:\'{"text":"two"}\'}'),
    (1900, 2514, 'jigsaw[facing=up]', '{attachement_type:"minecraft:bottom",target_pool:"minecraft:village/plains/houses",'
                                      'final_state:"minecraft:air"}'),
    (1900, 2514, 'jigsaw[facing=east]', '{attachement_type:"minecraft:empty",target_pool:"minecraft:empty"}'),
    (1900, 2514, 'lectern', '{Book:{id:"minecraft:written_book",Count:1b,tag:{title:"L",author:"A",pages:[\'"x"\']}},Page:0}'),
    (2100, 2514, 'beehive', '{Bees:[{EntityData:{id:"minecraft:bee"},TicksInHive:3,MinOccupationTicks:600}]}'),
]
BLOCK_ENTITIES = [b for b in BLOCK_ENTITIES if b]

# Plain blocks whose states changed: (since, until, command tail after "setblock x y z")
BLOCKS = [
    (1900, 2500, 'cobblestone_wall[east=true,west=false,north=true,south=false]'),
    (1900, 2500, 'andesite_wall[east=true,west=true,north=false,south=false,up=true]'),
    (1631, 2500, 'redstone_wire[east=none,west=none,north=none,south=none,power=0]'),
    (1631, 2500, 'redstone_wire[east=up,west=none,north=none,south=none,power=3]'),
    (1631, 2500, 'redstone_wire[east=none,west=none,north=side,south=none,power=1]'),
    (1631, 2500, 'oak_sign[rotation=3]'),
    (1631, 1900, 'stone_slab[type=bottom]'),
    (1631, 2100, 'bee_nest') if False else None,
]
BLOCKS = [b for b in BLOCKS if b]

ITEMS = [
    (1631, 2514, '{id:"minecraft:white_banner",Count:1b,tag:{display:{Name:\'{"translate":"block.minecraft.illager_banner","color":"gold"}\','
                 'Lore:[\'first\',\'{"text":"second"}\']},BlockEntityTag:{Patterns:[{Pattern:"cre",Color:15}]}}}'),
    (1631, 2514, '{id:"minecraft:player_head",Count:1b,tag:{SkullOwner:{Id:"%s",Name:"Notch"}}}' % U2),
    (1631, 2514, '{id:"minecraft:diamond_sword",Count:1b,tag:{AttributeModifiers:[{AttributeName:"generic.attackDamage",Name:"x",Amount:2.5d,'
                 'Operation:0,UUIDMost:41L,UUIDLeast:42L,Slot:"mainhand"}]}}'),
    (1631, 2514, '{id:"minecraft:filled_map",Count:1b,tag:{map:0}}'),
    (1631, 2514, '{id:"minecraft:cactus_green",Count:1b}') if False else None,
    (1631, 2514, '{id:"minecraft:stone_slab",Count:2b}') if False else None,
]
ITEMS = [i for i in ITEMS if i]


def entities(version):
    return [(kind, fields) for since, until, kind, fields in ENTITIES if since <= version < until]


def block_entities(version):
    return [(block, nbt) for since, until, block, nbt in BLOCK_ENTITIES if since <= version < until]


def blocks(version):
    return [block for since, until, block in BLOCKS if since <= version < until]


def items(version):
    return [stack for since, until, stack in ITEMS if since <= version < until]
