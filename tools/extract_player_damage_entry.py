#!/usr/bin/env python3
"""Pure-move the player equipment damage continuation out of game.rs."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root / "crates/horizon-client/src/game.rs"
text = path.read_text()
start = text.index("    /// Run equipment continuations with the acting player")
end = text.index("    /// `Player.hurtArmor`", start)
block = text[start:end]
path.write_text(text[:start] + text[end:])
(root / "crates/horizon-client/src/game/items/player_damage.rs").write_text(
    "use super::Game;\n\nimpl Game<'_> {\n" + block + "}\n"
)
