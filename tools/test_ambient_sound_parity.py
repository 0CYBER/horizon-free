#!/usr/bin/env python3
"""Compare the local player's ambient sounds with the vanilla 26.3 client.

The oracle runs the real `LocalPlayer.updateIsUnderwater` (enter/exit sounds
seeded from the level random, the `UnderLiquidAmbientSoundInstance`),
`UnderwaterAmbientSoundHandler`, `BubbleColumnAmbientSoundHandler` and
`BiomeAmbientSoundsHandler` over the real
`SoundEngine` (see VanillaSoundRig), then `SoundManager.tick`, once per client
tick. The `AMBIENT_SOUNDS` value (loops, moods and additions from the biomes
and dimensions of the data pack plus faster custom moods), the player's
position, eye height, eyes in water, spectator and removed flags and the
level's brightness are script inputs. Every step prints the level random's
next long (so every draw is compared), moodiness, the underwater state and
delay, the handler's loop instances (fade, direction, stopped, active), each
playing instance and the subtitle notifications. Horizon's `Ambience` and
`SoundEngine` replay the scripts.
"""
import json
import random

try:
    from . import audio_harness as h, lab
except ImportError:
    import audio_harness as h
    import lab


def settings():
    """Every AMBIENT_SOUNDS value in the data pack."""
    data = h.RESOURCES / 'data/minecraft'
    values = []
    for path in sorted(list((data / 'worldgen/biome').glob('*.json')) + list((data / 'dimension_type').glob('*.json'))):
        v = json.loads(path.read_text()).get('attributes', {}).get('minecraft:audio/ambient_sounds')
        if v is None:
            continue
        additions = v.get('additions', [])
        if isinstance(additions, dict):
            additions = [additions]
        mood = v.get('mood')
        values.append(dict(loop=v.get('loop'), mood=None if mood is None else (mood['sound'], mood['tick_delay'], mood['block_search_extent'], mood['offset']),
                           additions=[(a['sound'], a['tick_chance']) for a in additions]))
    return values


class Script:
    def __init__(self):
        self.lines = []

    def add(self, *parts):
        self.lines.append(' '.join(map(str, parts)))

    def sounds(self, value):
        mood = '-' if value['mood'] is None else '{},{},{},{}'.format(value['mood'][0], value['mood'][1], value['mood'][2], lab.dbits(value['mood'][3]))
        self.add('sounds', value['loop'] or '-', mood, *[f'{s},{lab.dbits(c)}' for s, c in value['additions']])

    def player(self, at=(0.5, 64.0, 0.5), eye=1.62, water=0, spectator=0, removed=0, mode=0, height=1.8, columns=()):
        self.add('player', *map(lab.dbits, at), h.bits(eye), water, spectator, removed, mode, h.bits(height),
                 *[f'{x},{y},{z},{d}' for x, y, z, d in columns])

    def tick(self, n=1):
        for _ in range(n):
            self.add('tick')


def scripted(s, values):
    by_loop = [v for v in values if v['loop']]
    cave = next(v for v in values if v['mood'] and v['mood'][0] == 'minecraft:ambient.cave' and not v['loop'])
    s.add('start', 1)
    s.sounds(cave)
    s.player(mode=1)
    s.tick(6100)
    s.player((100.25, -20.0, -7.75), mode=2)
    s.tick(400)
    s.player((3.5, 12.0, 3.5), mode=3)
    s.tick(400)
    # Biome loops cross-fading, back and forth.
    s.add('start', 2)
    for v in by_loop:
        s.sounds(v)
        s.tick(30)
    s.sounds(by_loop[0])
    s.tick(20)
    s.sounds(by_loop[1])
    s.tick(5)
    s.sounds(by_loop[0])
    s.tick(60)
    s.sounds(dict(loop=None, mood=None, additions=[]))
    s.tick(60)
    # Water: enter, the loop's fade, sub-sounds, exit, re-entering mid-fade, spectators.
    s.add('start', 3)
    s.sounds(cave)
    s.player(water=1, mode=0)
    s.tick(300)
    s.player(water=0)
    s.tick(10)
    s.player(water=1)
    s.tick(3)
    s.player(water=0)
    s.tick(2)
    s.player(water=1)
    s.tick(60)
    s.player(water=1, spectator=1)
    s.tick(5)
    s.player(water=0, spectator=1)
    s.tick(5)
    s.player(water=1, removed=1)
    s.tick(5)
    s.player(water=0)
    s.tick(25)
    # Silent ambient category: loops never start and linger.
    s.add('start', 4)
    s.add('volume', 8, h.bits(0.0))
    s.sounds(by_loop[0])
    s.tick(50)
    s.add('volume', 8, h.bits(1.0))
    s.sounds(by_loop[1])
    s.tick(50)
    s.sounds(by_loop[0])
    s.tick(50)
    # Bubble columns: entering, staying, the first tick, spectators, small boxes.
    s.add('start', 5)
    column = [(0, 64, 0, 1), (0, 65, 0, 1)]
    s.player((0.5, 64.0, 0.5), water=1, columns=column)
    s.tick(3)
    s.player((3.5, 64.0, 0.5), water=1, columns=column)
    s.tick(2)
    s.player((0.5, 64.0, 0.5), water=1, columns=column)
    s.tick(2)
    s.player((0.5, 64.0, 0.5), water=1, columns=[(0, 64, 0, 0), (1, 64, 0, 1)])
    s.tick(2)
    s.player((3.5, 64.0, 0.5))
    s.tick(2)
    s.player((0.9, 64.5, 0.5), water=1, height=0.6, columns=[(1, 64, 0, 0), (0, 64, 0, 1)])
    s.tick(2)
    s.player((3.5, 64.0, 0.5))
    s.tick(1)
    s.player((0.5, 63.7, 0.5), spectator=1, columns=column)
    s.tick(2)


def fuzz(s, rng, values, steps):
    s.add('start', rng.getrandbits(63))
    for _ in range(steps):
        op = rng.random()
        if op < 0.05:
            v = rng.choice(values + [dict(loop=None, mood=None, additions=[])])
            if rng.random() < 0.5 and v['mood']:
                v = dict(v, mood=(v['mood'][0], rng.choice([20, 60, 150]), rng.choice([1, 4, 8, 16]), rng.choice([0.0, 2.0, 5.5])))
            s.sounds(v)
        elif op < 0.12:
            at = (rng.uniform(-3e4, 3e4) if rng.random() < 0.2 else rng.uniform(-200, 200), rng.uniform(-64, 320), rng.uniform(-200, 200))
            columns = []
            if rng.random() < 0.3:
                bx, by, bz = int(at[0] // 1), int(at[1] // 1), int(at[2] // 1)
                columns = [(bx + rng.randrange(-1, 2), by + rng.randrange(-1, 3), bz + rng.randrange(-1, 2), rng.randrange(2)) for _ in range(rng.randrange(1, 4))]
            s.player(at, rng.choice([1.62, 1.27, 0.4, 0.2]), int(rng.random() < 0.4), int(rng.random() < 0.05), int(rng.random() < 0.03), rng.randrange(4),
                     rng.choice([1.8, 1.5, 0.6]), columns)
        elif op < 0.13:
            s.add('volume', rng.choice([0, 8]), h.bits(rng.choice([0.0, 1.0, 0.5])))
        else:
            s.tick(rng.randrange(1, 40))


def fixtures():
    values = settings()
    s = Script()
    scripted(s, values)
    rng = random.Random(0x414d4249)
    for _ in range(40):
        fuzz(s, rng, values, 150)
    return s.lines


def main():
    lines = fixtures()
    matched, report = h.run_harness(
        'ambient', 'VanillaAmbientOracle', 'ambient_fixture', ''.join(l + '\n' for l in lines),
        extra_sources=[h.lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSoundRig.java',
                       h.lab.ROOT / 'crates/horizon-client/src/ambience.rs',
                       h.lab.ROOT / 'crates/horizon-client/examples/probe/audio_common/mod.rs'],
        java_args=[h.RESOURCES], rust_args=[h.RESOURCES],
        scope='Real LocalPlayer.updateIsUnderwater, UnderwaterAmbientSoundHandler and BiomeAmbientSoundsHandler (loop cross-fades, additions, '
              'cave mood sampling, moodiness and the mood sound position) with UnderLiquidAmbientSoundInstance/UnderLiquidSubSound/LoopSoundInstance '
              'ticking in the real SoundEngine: every level random draw, handler state, playing instance and notification per tick. The '
              'AMBIENT_SOUNDS value, eyes-in-water state and brightness are inputs. BubbleColumnAmbientSoundHandler runs on a scripted block world.')
    print(f'{matched:,} ambient lines match the pinned client ({len(lines):,} steps): {report}', flush=True)


if __name__ == '__main__':
    main()
