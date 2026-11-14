#!/usr/bin/env python3
"""Move the original hostile sensor out of the capped brain, preserving scope."""
from pathlib import Path
p = Path('crates/horizon-client/src/mobs/villager_brain.rs')
s = p.read_text()
a = s.index('/// `VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`.')
f = s.index('fn sense_hostiles(', a)
b = s.index('{', f) + 1
depth = 1
while depth:
    depth += (s[b] == '{') - (s[b] == '}')
    b += 1
b += 1
body = s[a:b]
assert 'neoforge_mob_maps::current()' in body
out = p.with_suffix('') / 'hostile_sensor.rs'
assert not out.exists()
out.parent.mkdir(exist_ok=True)
out.write_text('// Included in villager_brain.rs to retain its private sensor scope.\n' + body)
p.write_text(s[:a] + 'include!("villager_brain/hostile_sensor.rs");\n' + s[b:])
