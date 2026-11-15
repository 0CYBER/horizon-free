#!/usr/bin/env python3
"""Distinguish engine loot callback invokers from API hook targets."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATHS = [
    "crates/horizon-world/src/loot.rs",
    "crates/horizon-world/tests/native_load_hooks.rs",
    "crates/horizon-items/src/items.rs",
    "crates/horizon-client/src/game/native_hooks/shared.rs",
]

for name in PATHS:
    path = ROOT / name
    source = path.read_text()
    for kind in ("function", "condition"):
        source = source.replace(f"native_loot_{kind}", f"native_invoke_loot_{kind}")
    path.write_text(source)
