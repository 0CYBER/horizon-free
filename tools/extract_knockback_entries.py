#!/usr/bin/env python3
"""Pure-move knockback implementations out of oversized gameplay modules."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root / "crates/horizon-client/src/mobs.rs"
text = path.read_text()
start = text.index("    /// LivingEntity.knockback(power")
end = text.index("    /// One game tick. Returns false", start)
block = text[start:end]
text = text[:start] + text[end:]
text = text.replace("mod world;", "mod world;\nmod knockback;")
path.write_text(text)
(path.parent / "mobs/knockback.rs").write_text("use super::*;\n\nimpl Mob {\n" + block + "}\n")
path = root / "crates/horizon-client/src/game/mobs/creatures/mod.rs"
text = path.read_text()
start = text.index("    /// `LivingEntity.knockback` on the player")
end = text.index("    /// A status effect from a mob", start)
method = text[start:end]
text = text[:start] + text[end:]
start = text.index("/// `LivingEntity.knockback` on `p`")
end = text.index("/// The instance a mob effect", start)
helper = text[start:end]
text = text[:start] + text[end:]
text += "\nmod knockback;\n"
path.write_text(text)
(path.parent / "knockback.rs").write_text("use super::*;\n\nimpl Game<'_> {\n" + method + "}\n\n" + helper)
