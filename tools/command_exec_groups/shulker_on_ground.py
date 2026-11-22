"""`shulker on ground`: a shulker with AI clears a loaded `OnGround` on its tick (#18737)."""

GROUP = 'shulker on ground'


def cases(cmd, setup):
    """`Shulker.setDeltaMovement` is empty, so its motion stays zero, but `LivingEntity.aiStep` still runs
    `travel`, whose `Entity.move(SELF, Vec3.ZERO)` meets no collision below: `setOnGroundWithMovement` sets
    `onGround` to false. A `NoAI` shulker keeps what it loaded. Read when summoned, tick by tick, and after
    its chunk is saved and loaded again."""
    x0, y0, z0 = 6752, 250, 6752
    # Entities tick once every chunk within 2 columns is loaded (ChunkMap.prepareEntityTickingChunk).
    chunk = [[(x0 >> 4) + dx, (z0 >> 4) + dz] for dx in range(-2, 3) for dz in range(-2, 3)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} stone', expect_result=256)]
    lanes = ['', 'OnGround:1b,', 'OnGround:0b,', 'OnGround:-1b,', 'NoAI:1b,OnGround:1b,']
    uuids = []
    for n, nbt in enumerate(lanes, 1):
        uuids.append(f'{18737:08x}-0000-0000-0000-{n:04x}00000001')
        ops.append(cmd(f'/summon shulker {x0 + 2 + n * 2}.5 {y0} {z0 + 8}.5 '
                       f'{{UUID:[I;18737,0,{n},1],{nbt}PersistenceRequired:1b}}', expect_result=1))

    def reads():
        for uuid in uuids:
            ops.append(cmd(f'/data get entity {uuid} OnGround'))

    reads()
    for _ in range(3):
        ops.append({'op': 'tick', 'n': 1})
        reads()
    # A real save and load: the oracle's ticks need their spare time to unload the chunk.
    ops += [cmd(f'/forceload remove {x0} {z0}', False), {'op': 'tick', 'n': 60, 'allow_chunk_io': True},
            cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk}]
    reads()
    ops.append({'op': 'tick', 'n': 1})
    reads()
    return ops + setup(f'/kill @e[type=shulker,x={x0},y={y0 - 8},z={z0},dx=16,dy=16,dz=16]',
                       f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
