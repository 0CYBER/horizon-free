"""Server orb fire: saved `Fire` and the 20-tick burn of `Entity.baseTick`, tick by tick."""
try:
    from .command_exec_corpus import cmd, setup
except ImportError:
    from command_exec_corpus import cmd, setup

# (name, Fire, Health): `Entity.baseTick` burns an orb with Fire > 0 once on every tick where
# the counter is a multiple of 20 (`hurtServer(onFire, 1)`), then counts it down by one.
ACTORS = [('burn100', 100, 5), ('burn25', 25, 5), ('none', 0, 5), ('neg', -5, 5),
          ('big', 32760, 5), ('last', 20, 1)]
CHECKPOINTS = [0, 1, 2, 6, 7, 21, 41, 61, 80, 81, 82]


def burnt(fire, health, ticks):
    """Fire, health and whether the orb lives, after `ticks` server ticks (independent model)."""
    for _ in range(ticks):
        if fire > 0:
            if fire % 20 == 0:
                health -= 1
            fire -= 1
            if health <= 0:
                return fire, health, False
        fire = max(fire, 0)  # Entity.move: applyEffectsFromBlocks puts an unlit entity at 0
    return fire, health, True


def orb_fire():
    out = setup('/tick freeze', '/gamerule minecraft:spawn_mobs false',
                '/gamerule minecraft:advance_time false', '/weather clear', '/gamemode spectator',
                '/tp @s 24.5 210 24.5 0 0')
    out += [cmd('/forceload add 0 0 31 31', expect_result=4), {'op': 'settle'}]
    for x in [0, 16]:
        for z in [0, 16]:
            out.append(cmd(f'/execute if loaded {x} 200 {z}', expect_result=1))
    out.append(cmd('/fill 0 199 0 31 199 31 stone', expect_result=1024))
    out += setup('/scoreboard objectives add orb_audit dummy')
    for n, (name, fire, health) in enumerate(ACTORS, 1):
        out.append(cmd(f'/summon experience_orb {1.5 + 4 * n} 200 3.5 '
                       f'{{UUID:[I;306,0,0,{n}],Tags:["h306_{name}"],Health:{health}s,Age:0s,'
                       f'Value:{n}s,Count:1,Fire:{fire}s,Motion:[0d,0d,0d]}}', expect_result=1))
    out += setup('/execute store result score start orb_audit run time query gametime')
    done = 0
    for at in CHECKPOINTS:
        if at > done:
            out += [cmd(f'/tick step {at - done}', expect_result=1), {'op': 'tick', 'n': at - done}]
            done = at
        out += setup('/execute store result score elapsed orb_audit run time query gametime',
                     '/scoreboard players operation elapsed orb_audit -= start orb_audit')
        out.append(cmd('/scoreboard players get elapsed orb_audit', expect_result=at))
        for n, (name, fire, health) in enumerate(ACTORS, 1):
            uuid = f'00000132-0000-0000-0000-{n:012x}'
            f, h, alive = burnt(fire, health, at)
            op = cmd(f'/execute if entity {uuid}')
            op['expect_callback'] = [alive, int(alive)]
            out.append(op)
            if not alive:
                out.append(cmd(f'/execute unless entity {uuid}', expect_result=1))
                continue
            out.append(cmd(f'/execute as {uuid} if entity @s[type=experience_orb,tag=h306_{name},'
                           f'nbt={{Health:{h}s,Fire:{f}s,Age:{at}s,Value:{n}s,Count:1}}]', expect_result=1))
            for field in ['Fire', 'Health', 'Age']:
                out.append(cmd(f'/data get entity {uuid} {field}'))
    return out
