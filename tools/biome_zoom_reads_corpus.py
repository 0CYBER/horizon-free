"""Positional biome reads through `BiomeManager.getBiome`'s zoom (#14980).

`LevelReader.getBiome(BlockPos)` picks the nearest of the eight jittered quart
corners around a block, so a block near a biome border can read the
neighbouring cell. Seed 1, at x 4343 z 4104: the block's own quart cell is
plains, the zoom picks the savanna next to it.
"""
from command_exec_corpus import cmd, setup

X, Z, Y = 4336, 4096, 300


def pig(x, z):
    """A dispensed pig spawn egg (`Pig.finalizeSpawn` -> `SpawnContext.create`)."""
    return [cmd(f'/setblock {x} {Y} {z} minecraft:dispenser[facing=south]'
                f'{{Items:[{{Slot:0b,id:"minecraft:pig_spawn_egg",count:1}}]}}', expect_result=1),
            cmd(f'/setblock {x} {Y + 1} {z} minecraft:redstone_block', expect_result=1)]


def biome_zoom_reads():
    ops = [*setup(f'/forceload add {X} {Z} {X + 15} {Z + 15}'),
           {'op': 'wait_chunks', 'chunks': [[X >> 4, Z >> 4]]},
           *setup(f'/fill {X} {Y - 1} {Z} {X + 15} {Y - 1} {Z + 15} minecraft:stone')]
    # Inside the blend band: the stored quart cell is plains, the zoom picks savanna.
    ops += [cmd(f'/execute if biome {X + 7} {Y} {Z + 8} minecraft:savanna', expect_result=1),
            cmd(f'/execute if biome {X + 7} {Y} {Z + 8} minecraft:plains'),
            # The source itself: plains away from the jitter.
            cmd(f'/execute if biome {X + 6} {Y - 2} {Z + 10} minecraft:plains', expect_result=1)]
    # The border pig lands at (4343.5, 300, 4104.5); the control in savanna proper.
    ops += pig(X + 7, Z + 7) + pig(X + 3, Z + 3)
    ops.append({'op': 'tick', 'n': 5})
    for x, z in [(X + 7, Z + 8), (X + 3, Z + 4)]:
        near = f'@e[type=minecraft:pig,x={x}.5,y={Y},z={z}.5,distance=..0.9,limit=1]'
        ops += [cmd(f'/execute if entity {near}', expect_result=1),
                cmd(f'/data get entity {near} variant')]
    return ops + setup(f'/kill @e[type=minecraft:pig,x={X},y={Y - 8},z={Z},dx=16,dy=24,dz=16]',
                       f'/fill {X} {Y - 1} {Z} {X + 15} {Y + 1} {Z + 15} minecraft:air',
                       f'/forceload remove {X} {Z} {X + 15} {Z + 15}')
