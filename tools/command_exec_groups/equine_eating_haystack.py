"""`equine eating haystack`: every equine loads and saves its eating state (#18677)."""

GROUP = 'equine eating haystack'


def cases(cmd, setup):
    """`AbstractHorse.readAdditionalSaveData` runs `setEating(getBooleanOr("EatingHaystack", false))` and
    `addAdditionalSaveData` writes `isEating()`, so a nonzero byte reads back as 1b. `AbstractHorse.aiStep` ends
    the eating after 50 ticks where `canEatGrass` (not a llama). Each stands on stone, so none starts grazing;
    the ones with AI stand in a pit. `Llama.isImmobile` is its eating: the llama with AI runs no goals, so it stays
    where it was summoned. Read when summoned and after 200 ticks."""
    x0, y0, z0 = 6624, 250, 6624
    # Entities tick once every chunk within 2 columns is loaded (ChunkMap.prepareEntityTickingChunk).
    chunk = [[(x0 >> 4) + dx, (z0 >> 4) + dz] for dx in range(-2, 3) for dz in range(-2, 3)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} stone', expect_result=256)]
    lanes = [('horse', '-1b', 'NoAI:1b,'), ('zombie_horse', '-1b', 'NoAI:1b,'), ('skeleton_horse', '2b', 'NoAI:1b,'),
             ('donkey', '0b', 'NoAI:1b,'), ('llama', '-1b', 'NoAI:1b,'), ('trader_llama', '1b', 'NoAI:1b,'),
             ('horse', '1b', ''), ('llama', '1b', '')]
    uuids = []
    for n, (kind, eating, extra) in enumerate(lanes, 1):
        # Two rows of four; the llama with AI in a third row, its pit clear of the horse's.
        x, z = (x0 + 3, z0 + 13) if n == 8 else (x0 + 2 + (n - 1) % 4 * 4, z0 + 2 + (n - 1) // 4 * 6)
        uuids.append(f'{18677:08x}-0000-0000-0000-{n:04x}00000001')
        if not extra:  # a pit of stone around the one that walks
            ops.append(cmd(f'/fill {x - 2} {y0} {z - 2} {x + 2} {y0 + 1} {z + 2} stone', expect_result=50))
            ops.append(cmd(f'/fill {x - 1} {y0} {z - 1} {x + 1} {y0 + 1} {z + 1} air', expect_result=18))
        ops.append(cmd(f'/summon {kind} {x}.5 {y0} {z}.5 '
                       f'{{UUID:[I;18677,0,{n},1],{extra}EatingHaystack:{eating},PersistenceRequired:1b}}',
                       expect_result=1))

    def reads():
        for uuid in uuids:
            ops.append(cmd(f'/data get entity {uuid} EatingHaystack'))
        ops.append(cmd(f'/data get entity {uuids[-1]} Pos'))  # the llama with AI

    reads()
    ops.append({'op': 'tick', 'n': 200})
    reads()
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 + 3} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
