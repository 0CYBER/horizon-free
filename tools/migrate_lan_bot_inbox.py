#!/usr/bin/env python3
"""Migrate lan_bot's synthetic packet tuples to owned InboxItem variants.

Run once from the repository root, then rustfmt the target. Only this fixture
uses push_back in this file; reject unexpected tuple credit or expression shapes.
"""
from pathlib import Path
import re

path = Path("crates/horizon-client/tests/lan_bot.rs")
source = path.read_text()
assert "guest::InboxItem," not in source, "migration already applied"
source = source.replace("        entity::EntityState,\n", "        entity::EntityState,\n        guest::InboxItem,\n", 1)
needle = ".push_back(("
count = 0
cursor = 0
while (start := source.find(needle, cursor)) != -1:
    opening = start + len(needle) - 1
    depth = 1
    closing = opening + 1
    while depth:
        character = source[closing]
        if character in "([{":
            depth += 1
        elif character in ")]}":
            depth -= 1
        closing += 1
    contents = source[opening + 1:closing - 1]
    assert contents.lstrip().startswith(("S::", "packet,")), contents
    contents, replaced = re.subn(r",(\s*)0(\s*,?\s*)$", r",\1None\2", contents)
    assert replaced == 1, contents
    replacement = "InboxItem::Packet(" + contents + ")"
    source = source[:opening] + replacement + source[closing:]
    cursor = opening + len(replacement)
    count += 1
source, patterns = re.subn(
    r"Some\(\((S::\w+\(_\)), _\)\)", r"Some(InboxItem::Packet(\1, _))", source
)
assert count == 21, count
assert patterns == 3, patterns
path.write_text(source)
print(f"Migrated {count} packet insertions and {patterns} inbox patterns in {path}")
