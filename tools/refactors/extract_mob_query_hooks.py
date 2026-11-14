#!/usr/bin/env python3
"""Extract the Game MobHooks implementation without changing its scope or behavior."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game/mobs/creatures/mod.rs"
target = source.with_name("mob_query_hooks.rs")
text = source.read_text()
marker = "impl horizon_client::mobs::MobHooks for Game<'_> {"
assert text.count(marker) == 1, "expected one unextracted MobHooks implementation"
assert not target.exists(), "refuse to overwrite an existing extraction"
start = text.index(marker)
end = text.index("\nimpl Game<'_> {", start)
block = text[start:end].rstrip()
assert block.endswith("}"), "expected the complete trait implementation"
target.write_text("// Included in creatures/mod.rs to preserve its existing scope.\n" + block + "\n")
source.write_text(text[:start] + 'include!("mob_query_hooks.rs");\n' + text[end:])
