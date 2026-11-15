#!/usr/bin/env python3
"""An armadillo's scute_time survives save and load, and a load without it keeps the constructor's draw.

`Armadillo()` draws `scuteTime = pickNextScuteDropTime()` (`nextInt(6000) + 6000`) after its brain's sensor
delays; `readAdditionalSaveData` writes `scute_time` only when the tag has it, and a reload's `makeBrain` draws
the sensor delays again but not the scute time. Armadillos are summoned with NoAI (so `customServerAiStep` does
not count the time down), one without `scute_time` and one with 4321, the world is saved and reloaded
(tools/entity_save_reload.py), and each is read with `/data get` on vanilla's server and Horizon's:

- `explicit`: 4321 live and reloaded.
- `drawn`: the drawn time scaled by 1/6000 is 1 (6000 to 11999, never 0), and copying it again onto the copy
  the setup took into command storage changes nothing, live and after the reload.
"""
try:
    from . import entity_save_reload as harness
except ImportError:
    import entity_save_reload as harness

SETUP = [
    'summon minecraft:armadillo 4.5 -60 4.5 {Tags:["drawn"],NoAI:1b,PersistenceRequired:1b}',
    'summon minecraft:armadillo 8.5 -60 4.5 {Tags:["explicit"],NoAI:1b,PersistenceRequired:1b,scute_time:4321}',
    'data modify storage horizon:armadillo drawn set from entity @e[tag=drawn,limit=1] scute_time',
]
PROBES = [
    ('explicit scute_time', 'data get entity @e[tag=explicit,limit=1] scute_time'),
    ('drawn scute_time / 6000', 'data get entity @e[tag=drawn,limit=1] scute_time 0.000166667'),
    ('drawn scute_time unchanged',
     'data modify storage horizon:armadillo drawn set from entity @e[tag=drawn,limit=1] scute_time'),
]


def main():
    return harness.main('armadillo', SETUP, PROBES, __doc__)


if __name__ == '__main__':
    raise SystemExit(main())
