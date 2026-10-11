#!/usr/bin/env python3
"""Compare sound event resolution with the vanilla 26.3 client.

The oracle builds the real `SoundManager` registry from the client asset root
(`SoundManager.prepare` + `Preparations.apply`: sounds.json, `replace`,
subtitles, file validation against the listed .ogg resources, event references
weighing as much as their event) and resolves seeded `SimpleSoundInstance`s:
`getOrResolve`, `getVolume`, `getPitch`, `Sound.getAttenuationDistance`, the
stream/preload/type fields and the instance random's next draw. Horizon's
`SoundRegistry` + `SoundEngine::resolve` replay the same inputs.
"""
import json
import random

try:
    from . import audio_harness as h
except ImportError:
    import audio_harness as h


def fixtures():
    sounds = json.loads((h.RESOURCES / 'assets/minecraft/sounds.json').read_text())
    rng = random.Random(0x534f554e44)
    volumes = [1.0, 0.5, 0.25, 2.0, 4.0, 0.0, 1e-3, 16.0]
    pitches = [1.0, 0.5, 2.0, 0.8, 1.2, 3.0, 0.1]
    lines = []
    for name in sorted(sounds):
        event = 'minecraft:' + name
        count = len(sounds[name].get('sounds', []))
        seeds = [0, 1, -1, 42] + [rng.getrandbits(64) - (1 << 63) for _ in range(max(4, 2 * count))]
        for seed in seeds:
            lines.append(f'{event} {seed} {h.bits(rng.choice(volumes))} {h.bits(rng.choice(pitches))}')
    for event in ['minecraft:intentionally_empty', 'minecraft:empty', 'minecraft:not.a.sound', 'horizon:missing', 'block.stone.break', 'music.game']:
        for seed in [0, 7, -99]:
            lines.append(f'{event} {seed} {h.bits(1.0)} {h.bits(1.0)}')
    return lines


def main():
    lines = fixtures()
    matched, report = h.run_harness(
        'sound-events', 'VanillaSoundEventOracle', 'sound_event_fixture', ''.join(l + '\n' for l in lines),
        java_args=[h.RESOURCES], rust_args=[h.RESOURCES],
        scope='Real SoundManager.prepare/Preparations.apply over the client asset root and SimpleSoundInstance.getOrResolve/getVolume/getPitch '
              'for every sounds.json event with seeded randoms (weighted choice, nested event references, MultipliedFloats volume/pitch, '
              'stream/preload/attenuation fields, EMPTY and INTENTIONALLY_EMPTY sentinels, draw counts) and the full registered event list.')
    print(f'{matched:,} sound event lines match the pinned client ({len(lines):,} resolutions): {report}', flush=True)


if __name__ == '__main__':
    main()
