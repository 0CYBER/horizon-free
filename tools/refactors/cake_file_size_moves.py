#!/usr/bin/env python3
"""Move baseline helpers and a cake test to make room for the cake hook."""
from pathlib import Path

remote = Path('crates/horizon-client/src/game/network/remote/mod.rs')
s = remote.read_text()
a = s.index('/// The client\'s world for a guess')
b = s.index('type DrySound', a)
c = s.index("impl<'w> DryLevel<'w>", b)
d = s.index('/// `InteractionHand` on the wire.', c)
struct = s[a:b].replace('struct DryLevel', 'pub(super) struct DryLevel').replace('    changed:', '    pub(super) changed:').replace('    sounds:', '    pub(super) sounds:')
impls = s[c:d].replace('    fn new(', '    pub(super) fn new(')
remote.with_name('dry_level.rs').write_text('use super::*;\n\n' + struct + impls)
remote.write_text(s[:a] + 'mod dry_level;\nuse dry_level::DryLevel;\n\n' + s[b:c] + s[d:])

tests = Path('crates/horizon-world/src/level/tests.rs')
s = tests.read_text()
a = s.index('#[test]\nfn a_candle_on_an_uneaten_cake')
new_test = s.find('#[test]\nfn cake_eligibility_is_asked_only_for_a_bite')
if new_test >= 0:
    a = min(a, new_test)
b = s.index('#[test]\nfn water_douses', a)
tests.with_name('cake_tests.rs').write_text('use super::*;\n\n' + s[a:b])
tests.write_text(s[:a] + '#[path = "cake_tests.rs"]\nmod cake_tests;\n\n' + s[b:])

game = Path('crates/horizon-client/src/game.rs')
s = game.read_text()
a = s.index("impl Game<'_> {\n    fn held(&self)")
b = s.index("impl Game<'_> {", a + 1)
chunk = s[a:b].replace('    fn held(', '    pub(super) fn held(')
Path('crates/horizon-client/src/game/items/held.rs').write_text('use super::*;\n\n' + chunk)
game.write_text(s[:a] + s[b:])

# Move the weather queries together; their callers remain in the game module.
s = game.read_text()
a = s.index("    fn dark_outside(&self)")
b = s.index("    fn held_mut(&mut self)", a)
chunk = s[a:b]
for name in ("dark_outside", "can_have_weather", "weather_levels"):
    chunk = chunk.replace("    fn " + name, "    pub(super) fn " + name)
Path('crates/horizon-client/src/game/world/weather_queries.rs').write_text(
    "use super::*;\n\nimpl Game<'_> {\n" + chunk + "}\n"
)
game.write_text(s[:a] + s[b:])
