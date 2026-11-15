#!/usr/bin/env python3
"""Move player hit-source wrappers verbatim out of oversized creatures/mod.rs."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = root / 'crates/horizon-client/src/game/mobs/creatures/mod.rs'
destination = root / 'crates/horizon-client/src/game/items/player_hit_sources.rs'
text = source.read_text()
start = text.index('    /// Damage of a given type to the player from a point.\n')
end = text.index('    /// `hurtServer` on the player.', start)
body = text[start:end]
destination.write_text('//! Player hit-source adapters.\nuse super::Game;\n\nimpl Game<\'_> {\n' + body + '}\n')
source.write_text(text[:start] + text[end:])
