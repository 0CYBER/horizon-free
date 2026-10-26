#!/usr/bin/env python3
"""Respawning in a hardcore world on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one hardcore
flat world vanilla generated first, with Alice an operator in survival. LocatorBarClient (a player made of
vanilla's protocol code) joins, runs /kill on itself and presses the death screen's button
(ServerboundClientCommandPacket PERFORM_RESPAWN). Vanilla's ServerGamePacketListenerImpl.handleClientCommand
respawns the player, then makes it a spectator because the server is hardcore. The game mode changes and respawns
Alice is sent in the death and respawn steps are compared exactly.
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'hardcore-respawn-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, hardcore='true', gamemode='survival')
SCRIPT = [
    ('death', ['cmd Alice kill @s']),
    ('respawn', ['respawn Alice']),
]


def shown(lines):
    """Alice's game mode changes and respawns in the death and respawn steps."""
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3
            and parts[1] in ('step=death', 'step=respawn') and parts[2] in ('gamemode', 'respawn')]


def check(vanilla, _):
    if 'Alice step=respawn gamemode 3' not in vanilla:
        return 'vanilla did not make the respawned player a spectator'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'A player dying and respawning in a hardcore world on a '
                               'dedicated server: the respawn and the game mode it is sent (spectator).')
    print(f'hardcore respawn: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
