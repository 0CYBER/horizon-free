"""A real WorkAtPoi restock must update and survive the open merchant menu."""
from command_exec_corpus import cmd


def cases():
    ops = []

    def c(command, result=None):
        ops.append(cmd(command, expect_result=result))

    def action(kind, **values):
        ops.append({'op': 'merchant_action', 'action': kind, **values})

    def menu(label, *, merchant=1, window=1, uses=12, demand=0,
             special=0, cost=4, payment=True, result=False):
        offers = [{'uses': uses, 'special': special, 'demand': demand, 'cost': cost}]
        ops.append({'op': 'merchant_state', 'label': label, 'offer_packets': True, 'expect_observation': {
            'window': window, 'merchant': [113, 1, 0, merchant],
            'player': [-1608632253, -450807400, -1786055591, -2146546415],
            'slots': [['minecraft:emerald', 20] if payment else None, None,
                      ['minecraft:glass', 3] if result else None],
            'carried': None, 'xp': 20, 'offers': offers, 'sent_offers': offers}})

    c('/tick freeze')
    ops += [cmd('/gamerule spawn_mobs false', record=False),
            cmd('/gamerule spawn_patrols false', record=False),
            cmd('/gamerule advance_time false', record=False),
            cmd('/gamerule advance_weather false', record=False)]
    c('/forceload add 0 0 31 31')
    ops.append({'op': 'wait_chunks', 'chunks': [[0, 0], [0, 1], [1, 0], [1, 1]]})
    ops += [{'op': 'tick', 'n': 40, 'pace_ms': 50},
            {'op': 'tick', 'n': 120, 'pace_ms': 50}]
    c('/fill 0 199 0 31 199 31 stone', 1024)
    c('/fill 0 200 0 31 204 31 air')
    c('/gamemode survival @s')
    c('/tp @s 8.5 200 5.5 0 0')
    c('/time set 2000')
    c('/setblock 8 200 8 brewing_stand')
    uuid = '00000071-0000-0001-0000-000000000001'
    nbt = ('{UUID:[I;113,1,0,1],Tags:["restock_witness"],NoAI:0b,NoGravity:1b,'
           'Silent:1b,PersistenceRequired:1b,Health:20f,attributes:[{id:"minecraft:movement_speed",base:0d}],'
           'VillagerData:{profession:"minecraft:cleric",level:2,type:"minecraft:plains"},Xp:20,'
           'Brain:{memories:{"minecraft:job_site":{value:{dimension:"minecraft:overworld",pos:[I;8,200,8]}}}},'
           'LastRestock:0L,RestocksToday:0,'
           'Offers:{Recipes:[{buy:{id:"minecraft:emerald",count:4},sell:{id:"minecraft:glass",count:3},'
           'uses:12,maxUses:12,rewardExp:0b,xp:1,priceMultiplier:0.05f,demand:0,specialPrice:0}]}}')
    c(f'/summon villager 8.5 200 7.5 {nbt}')
    c(f'/execute if entity @e[type=villager,tag=restock_witness,nbt={{UUID:[I;113,1,0,1],Health:20f,Pos:[8.5d,200d,7.5d]}}]', 1)
    c(f'/data get entity {uuid} Brain.memories."minecraft:job_site".value.pos', 3)
    c(f'/data get entity {uuid} Offers.Recipes[0].uses', 12)
    c('/item replace entity @s hotbar.0 with emerald 20', 1)
    action('open', uuid=uuid)
    action('select', index=0)
    menu('exhausted-open')
    # Restocking refreshes discounts from the current player's effects, not
    # only the Hero snapshot captured when this screen opened.
    c('/effect give @s hero_of_the_village 1000000 0 true')
    # The real Brain must pass WorkAtPoi's cooldown, random check and distance
    # gate. RestocksToday and demand below witness the actual restock.
    c('/tick step 800')
    ops.append({'op': 'tick', 'n': 800})
    c('/tick freeze')
    c(f'/execute if entity @e[tag=restock_witness,nbt={{Pos:[8.5d,200d,7.5d],RestocksToday:1}}]', 1)
    c(f'/data get entity {uuid} Offers.Recipes[0].uses')
    c(f'/data get entity {uuid} Offers.Recipes[0].demand', 12)
    menu('restocked-open', uses=0, demand=12, special=-1, cost=5, result=True)
    action('close')
    # These commands read the entity's saved NBT, not the menu clone.
    c(f'/data get entity {uuid} Offers.Recipes[0].specialPrice', -1)
    c(f'/data get entity {uuid} Offers.Recipes[0].uses')
    c(f'/data get entity {uuid} Offers.Recipes[0].demand', 12)
    c(f'/execute if entity @e[tag=restock_witness,nbt={{RestocksToday:1}}]', 1)
    action('open', uuid=uuid)
    menu('restocked-reopened', window=2, uses=0, demand=12, special=-1, cost=5, payment=False)
    action('close')
    # catchUpDemand refreshes prices even when no offers change: two past
    # restocks mean zero missed updates; unused stock means no new restock.
    c('/effect clear @s hero_of_the_village')
    c('/tp @s 16.5 200 5.5 0 0')
    c('/setblock 16 200 8 brewing_stand')
    catchup = (nbt.replace('[I;113,1,0,1]', '[I;113,1,0,2]')
               .replace('restock_witness', 'catchup_witness')
               .replace('pos:[I;8,200,8]', 'pos:[I;16,200,8]')
               .replace('LastRestock:0L,RestocksToday:0', 'LastRestock:-24000L,RestocksToday:2')
               .replace('uses:12,maxUses:12', 'uses:0,maxUses:12'))
    uuid = '00000071-0000-0001-0000-000000000002'
    c(f'/summon villager 16.5 200 7.5 {catchup}')
    c(f'/execute if entity @e[tag=catchup_witness,nbt={{UUID:[I;113,1,0,2],Pos:[16.5d,200d,7.5d],RestocksToday:2}}]', 1)
    c(f'/data get entity {uuid} Brain.memories."minecraft:job_site".value.pos', 3)
    action('open', uuid=uuid)
    action('select', index=0)
    menu('catchup-open', merchant=2, window=3, uses=0, result=True)
    c('/effect give @s hero_of_the_village 1000000 0 true')
    c('/tick step 800')
    ops.append({'op': 'tick', 'n': 800})
    c('/tick freeze')
    c(f'/execute if entity @e[tag=catchup_witness,nbt={{Pos:[16.5d,200d,7.5d],RestocksToday:0}}]', 1)
    c(f'/execute unless data entity {uuid} Offers.Recipes[0].uses', 1)
    c(f'/execute unless data entity {uuid} Offers.Recipes[0].demand', 1)
    menu('catchup-refreshed', merchant=2, window=3, uses=0, special=-1, cost=3, result=True)
    action('close')
    c(f'/data get entity {uuid} Offers.Recipes[0].specialPrice', -1)
    action('open', uuid=uuid)
    menu('catchup-reopened', merchant=2, window=4, uses=0, special=-1, cost=3, payment=False)
    action('close')
    ops.append({'op': 'merchant_save'})
    return ops
