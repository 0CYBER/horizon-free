#!/usr/bin/env python3
"""Commands run by a dead player and commands that select one, on a dedicated server, against vanilla's.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival. LocatorBarClient (a player made of vanilla's protocol
code) runs /kill on itself and stays on the death screen. A dead player keeps its connection and its `ServerPlayer`:
it still sends commands, and its commands still run as it (`CommandSourceStack.getEntity`). Whether a selector finds
it depends on the form (EntitySelectorParser.parseSelector, EntitySelector.findEntities/findPlayers): only `@e` and
`@n` add `Entity::isAlive`; `@s` is the source's entity, a name and a players-only UUID look in the player list, a
world-limited `@a[distance=..]` and an entity UUID look in the level, which no longer holds the body once
LivingEntity.tickDeath removed it at its twentieth tick.

A few ticks after her death, while her body lies there, Alice asks the forms whose answer changes with its removal;
after the removal she asks every form, then respawns. Her chat replies to those questions are compared exactly. The kill's own step is not compared: Horizon
applies a player's death on the player's next tick, after the command's feedback, and vanilla inside the command.
"""
import hashlib
import sys
import uuid

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'dead-player-commands-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')


def offline_uuid(name):
    """UUIDUtil.createOfflinePlayerUUID: UUID.nameUUIDFromBytes of "OfflinePlayer:<name>"."""
    digest = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode()).digest())
    digest[6] = (digest[6] & 0x0f) | 0x30
    digest[8] = (digest[8] & 0x3f) | 0x80
    return str(uuid.UUID(bytes=bytes(digest)))


ALICE = offline_uuid('Alice')
# Each selector form, as an entity test and as an entity argument (EntityArgument.entity: one entity at most).
FORMS = [('@s', '@s'), ('@a', '@a[limit=1]'), ('@a[distance=..5]', '@a[distance=..5,limit=1]'), ('@p', '@p'),
         ('@r', '@r'), ('@e[type=player]', '@e[type=player,limit=1]'), ('@n[type=player]', '@n[type=player]'),
         ('Alice', 'Alice'), (ALICE, ALICE)]
PROBES = ([f'cmd Alice execute if entity {test}' for test, _ in FORMS]
          + [f'cmd Alice data get entity {single} Health' for _, single in FORMS]
          # EntityArgument.getPlayer: findSinglePlayer, the player list (a UUID is no player argument).
          + [f'cmd Alice experience query {who} levels' for who in ('@s', '@p', '@r', 'Alice')]
          # The dead sender's own source: where it runs.
          + ['cmd Alice data get entity @s Pos', 'cmd Alice data get entity @s Dimension'])
# While the body lies there, only the forms whose answer changes with its removal are asked: in one run Horizon took
# about a tick for each of a batch of commands, so more of them could reach past the twentieth tick.
BODY_PROBES = ['cmd Alice execute if entity @a[distance=..5]', f'cmd Alice execute if entity {ALICE}',
               'cmd Alice data get entity @s Health', 'cmd Alice data get entity Alice Health']
# A step lasts one tick of the server's clock, plus the client's round trips: the body is still there one step after
# the kill and gone (its twentieth tick) twenty-five steps later.
SCRIPT = [
    ('setup', ['cmd Alice tp @s 0.5 -60 0.5']),
    # The kill's own chat is recorded, not compared (see above).
    ('death', ['chat on', 'cmd Alice kill @s']),
    ('dead', BODY_PROBES),
    ('wait', ['chat off']),
    *[('wait', []) for _ in range(24)],
    ('removed', ['chat on', *PROBES]),
    ('respawn', ['chat off', 'respawn Alice']),
]


def shown(lines):
    """Alice's chat while dead, with the body there and after its removal."""
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3
            and parts[1] in ('step=dead', 'step=removed') and parts[2] == 'chat']


def check(vanilla, _):
    for step, probes in (('dead', BODY_PROBES), ('removed', PROBES)):
        if sum(line.startswith(f'Alice step={step} chat') for line in vanilla) != len(probes):
            return f'vanilla answered not every probe in step {step}'
    # A world-limited selector finds the body only while the level holds it.
    limited = 'commands.execute.conditional.pass'
    dead = [line for line in vanilla if line.startswith('Alice step=dead chat')]
    removed = [line for line in vanilla if line.startswith('Alice step=removed chat')]
    if limited not in dead[0] or limited in removed[2]:
        return 'the probes did not land before and after the body was removed'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'A dead player on a dedicated server: the commands it runs '
                               'and the selectors that find it, before and after its body is removed.',
                               step_seconds=0.05)
    print(f'dead player commands: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
