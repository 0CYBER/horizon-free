#!/usr/bin/env python3
"""Expand the four loot test groups so daily source discovery sees their attributes."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
path = root / "crates/horizon-client/src/game/world/container_loot/tests.rs"
source = path.read_text()
start = source.index("macro_rules! groups {")
invocation = source.index("groups! {", start)
body = source[invocation + len("groups! {"):].strip()
assert body.endswith("}")
body = body[:-1]
groups = re.findall(r"(\w+):\s*\[([^]]*)\],", body)
assert len(groups) == 4
assert not re.sub(r"\w+:\s*\[[^]]*\],", "", body).strip()
expanded = []
for group, entries in groups:
    names = [name.strip() for name in entries.split(",") if name.strip()]
    assert all(re.fullmatch(r"\w+", name) for name in names)
    lines = [
        "#[test]", '#[ignore = "daily"]', f"fn {group}() {{",
        f'    if in_child("{group}") {{', "        return;", "    }",
        "    run_group(&[",
    ]
    lines += [f'        ("{name}", {name} as fn()),' for name in names]
    lines += ["    ]);", "}"]
    expanded.append("\n".join(lines))
path.write_text(source[:start] + "\n\n".join(expanded) + "\n")
