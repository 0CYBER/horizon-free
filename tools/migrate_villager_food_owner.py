#!/usr/bin/env python3
"""Mechanical first pass for the villager food owner (arismoko/horizon#3546; run from repo root).

Hand corrections for the registry owners (bindings, owner fields, fixtures) follow
separately. Every `Mob::seen` snapshot and the villager food calls gain an item
registry argument here:

- in the game's files, the `Game` the line already reads (`self.items()` or
  `game.items()`), else a bare `items` the hand pass binds;
- in the client library's mob tests, an explicit empty registry (no item is
  villager food), which fixtures that need food replace by hand;
- the villager food helpers (`food_points`, `Ext`'s food counts, `can_breed`,
  `villager::seen`) a bare `items` the hand pass binds.
"""
import re
from pathlib import Path

ROOT = Path("crates/horizon-client")
GAME = ROOT / "src/game"
EXAMPLES = ROOT / "examples"
# A `Mob::seen` call: its player argument is one of these (`MobWorld::seen` takes an id).
MOB_SEEN = re.compile(r"\.seen\((None|Some\(PLAYER_UUID\)|game\.level\.player_uuid|player)\)")
# The game's `creatures::seen(m, player)` helper, by path or imported.
HELPER_SEEN = re.compile(
    r"((?:super::|crate::game::)*creatures::seen|(?<![\w.:])(?:super::)?seen)"
    r"\(((?:&)?(?:m|mob|other|stale|mobs\[i\])), "
    r"(uuid|player_uuid|player|shared\.uuid|self\.level\.player_uuid|game\.level\.player_uuid|None)\)"
)
FOOD = [
    (re.compile(r"Ext::food_in_inventory\)"), "|e| e.food_in_inventory(items))"),
    (re.compile(r"Ext::wants_more_food\)"), "|e| e.wants_more_food(items))"),
    (re.compile(r"Ext::has_excess_food\)"), "|e| e.has_excess_food(items))"),
    (re.compile(r"\.(food_in_inventory|eat_and_digest|has_excess_food|wants_more_food)\(\)"),
     r".\1(items)"),
    (re.compile(r"(?<![\w.])((?:super::)*(?:villager::)?food_points)\((?!item: )"), r"\1(items, "),
    (re.compile(r"(?<![\w.])((?:super::)*(?:villager::)?can_breed)\(m\)"), r"\1(m, items)"),
    (re.compile(r"(?<![\w.])(villager::seen)\(m\)"), r"\1(m, items)"),
]


def owner(path: Path, line: str) -> str:
    """The registry expression a snapshot on `line` of `path` reads."""
    if path.is_relative_to(GAME):
        if path.name == "mob_group.rs":
            return "shared.items"
        if "self." in line:
            return "self.items()"
        if "game." in line:
            return "game.items()"
        return "items"
    if path.is_relative_to(EXAMPLES):
        return "items"
    return "&crate::items::Items::empty()"


def migrate(path: Path) -> None:
    source = path.read_text()
    lines = source.split("\n")
    for i, line in enumerate(lines):
        items = owner(path, line)
        line = MOB_SEEN.sub(lambda m: f".seen({m[1]}, {items})", line)
        line = HELPER_SEEN.sub(lambda m: f"{m[1]}({m[2]}, {m[3]}, {items})", line)
        for pattern, replacement in FOOD:
            line = pattern.sub(replacement, line)
        lines[i] = line
    migrated = "\n".join(lines)
    if migrated != source:
        path.write_text(migrated)


for path in sorted([*ROOT.joinpath("src").rglob("*.rs"), *EXAMPLES.rglob("*.rs")]):
    migrate(path)
