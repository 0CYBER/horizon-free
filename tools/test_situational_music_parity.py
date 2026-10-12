#!/usr/bin/env python3
"""Compare the situational music choice with the vanilla 26.3 client.

The oracle calls the real `Minecraft.getSituationalMusic` and
`getMusicVolume` with the open screen (none or the credits), the boss
overlay's bars (none, darkening, music), the player (absent, or in a dimension
with creative abilities and an underwater state) and a real camera
`EnvironmentAttributeProbe` over the dimension and biome attribute layers of
the vanilla data pack. Horizon's `Situation` with `MusicTables` answers the
same, for every dimension type x biome and every combination of the rest.
"""
import itertools

try:
    from . import audio_harness as h
except ImportError:
    import audio_harness as h


def fixtures():
    data = h.RESOURCES / 'data/minecraft'
    biomes = sorted('minecraft:' + p.stem for p in (data / 'worldgen/biome').glob('*.json'))
    dimensions = ['minecraft:overworld', 'minecraft:the_nether', 'minecraft:the_end']
    lines = []
    for screen, boss in itertools.product(['none', 'win'], ['none', 'dark', 'music', 'dark+music']):
        lines.append(f'{screen} 0 minecraft:overworld minecraft:plains {boss} 0 0 0')
    for d, b in itertools.product(dimensions, biomes):
        for screen, boss, instabuild, mayfly, water in itertools.product(['none', 'win'], ['none', 'dark', 'music'], '01', '01', '01'):
            lines.append(f'{screen} 1 {d} {b} {boss} {instabuild} {mayfly} {water}')
    return lines


def main():
    lines = fixtures()
    matched, report = h.run_harness(
        'situational-music', 'VanillaSituationalMusicOracle', 'situational_music_fixture', ''.join(l + '\n' for l in lines),
        extra_sources=[h.lab.ROOT / 'crates/horizon-client/src/music.rs'], rust_args=[h.RESOURCES],
        scope='Real Minecraft.getSituationalMusic/getMusicVolume: screen music (credits), menu without a player, End boss music from '
              'the boss overlay, BACKGROUND_MUSIC and MUSIC_VOLUME through a real camera EnvironmentAttributeProbe, creative abilities '
              '(instabuild && mayfly) and underwater selection, for every dimension x biome.')
    print(f'{matched:,} situational music lines match the pinned client: {report}', flush=True)


if __name__ == '__main__':
    main()
