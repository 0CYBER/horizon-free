#!/usr/bin/env python3
"""An ender pearl in flight when its thrower dies, on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival. LocatorBarClient (a player made of vanilla's protocol
code) throws an ender pearl straight up (ServerboundUseItemPacket), runs /kill on itself while the pearl flies,
presses the death screen's button (PERFORM_RESPAWN) and waits for the pearl to come down, with the rule
ender_pearls_vanish_on_death true: ThrownEnderpearl.tick discards a dead owner's pearl, so the respawned player is
never teleported. The teleports, game mode changes and respawns Alice is sent from the throw on are compared
exactly.
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'pearl-death-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')
SCRIPT = [
    ('setup', ['cmd Alice gamerule ender_pearls_vanish_on_death true', 'cmd Alice give @s minecraft:ender_pearl']),
    # Straight up: it comes down about four seconds later, near where it was thrown.
    ('throw', ['use Alice -90', 'cmd Alice kill @s']),
    ('respawn', ['respawn Alice']),
    ('landed', []),
]


def shown(lines):
    """Alice's teleports, game mode changes and respawns from the throw on."""
    steps = {f'step={step}' for step, _ in SCRIPT if step != 'setup'}
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] in steps
            and parts[2] in ('teleport', 'gamemode', 'respawn')]


def check(vanilla, vanilla_all):
    killed = any(l.startswith('Alice step=throw killed') for l in vanilla_all)
    if not killed or 'Alice step=respawn respawn minecraft:overworld' not in vanilla:
        return 'vanilla did not kill and respawn Alice'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'An ender pearl in flight when its thrower dies and '
                               'respawns on a dedicated server, with ender_pearls_vanish_on_death true: the pearl '
                               'vanishes and never teleports the player.')
    print(f'pearl death: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
