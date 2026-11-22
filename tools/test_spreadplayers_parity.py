#!/usr/bin/env python3
"""Spread a player below the Nether roof in terrain the command must load."""
import sys

import test_command_exec_parity as driver
from command_exec_corpus import cmd, group_setup


def groups():
    return [('setup', group_setup()), ('spreadplayers unloaded Nether', [
        {**cmd('/execute in minecraft:the_nether run spreadplayers -800 800 10 50 under -1 false @s'),
         'expect_callback': [False, 0]},
        cmd('/execute in minecraft:the_nether unless loaded -800 90 800', expect_result=1),
        cmd('/execute in minecraft:the_nether run spreadplayers -800 800 10 50 under 100 false @s',
            expect_result=1),
        # Each game's RNG can pick a different column. Compare the dimension,
        # bounds and supporting block of the resulting landing instead.
        cmd('/execute at @s if dimension minecraft:the_nether '
            'if entity @s[x=-850,y=1,z=750,dx=101,dy=98,dz=101]', expect_result=1),
        cmd('/execute at @s if block ~ ~-1 ~ #minecraft:entities_can_teleport_to', expect_result=1),
    ])]


if __name__ == '__main__':
    if '--work-dir' not in sys.argv:
        sys.argv += ['--work-dir', str(driver.lab.LOCAL / 'spreadplayers-parity')]
    driver.corpus_module.groups = groups
    driver.main()
