#!/usr/bin/env python3
"""Make installation side explicit at the dependency conversion boundary.

The mechanical pass uses Both for context-free metadata/build validation.
Runtime consumers are then corrected individually to their actual installation.
"""
from pathlib import Path

files = [
    "crates/horizon-client/src/game/modding/mods/mod.rs",
    "crates/horizon-client/src/game/modding/api_values/package_snapshot.rs",
    "crates/horizon-mod-wasm/src/lib.rs",
    "crates/horizon-mod-host/src/loading.rs",
    "crates/horizon-mod-package/src/dependencies.rs",
    "launcher/src/native_mods.rs",
    "launcher/src/native_mods/internals_sync.rs",
]
for filename in files:
    path = Path(filename)
    text = path.read_text()
    positions = []
    start = 0
    while (index := text.find("from_manifest(", start)) != -1:
        start = index + len("from_manifest(")
        # Exclude the function declaration, not calls in tests or parsing.
        if text[max(0, index - 3):index] == "fn ":
            continue
        depth = 1
        cursor = start
        quote = None
        escaped = False
        while depth:
            char = text[cursor]
            if quote:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == quote:
                    quote = None
            elif char == '"':
                quote = char
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            cursor += 1
        positions.append(cursor - 1)
        start = cursor
    side = "crate::manifest::Side::Both" if path.name == "dependencies.rs" else "horizon_mod_package::manifest::Side::Both"
    for position in reversed(positions):
        text = text[:position] + ", " + side + text[position:]
    path.write_text(text)
