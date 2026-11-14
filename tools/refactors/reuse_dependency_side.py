#!/usr/bin/env python3
"""Use the existing installation Side vocabulary for manifest load rules.

Run from the repository root. This migration removes the duplicate enum; it
intentionally does not leave an alias for the unmerged contributor spelling.
"""
from pathlib import Path

paths = [
    Path("crates/horizon-mod-package/src/manifest.rs"),
    Path("crates/horizon-mod-package/src/dependencies.rs"),
    Path("crates/horizon-mod-host/src/loading.rs"),
]
for path in paths:
    source = path.read_text()
    if path.name == "manifest.rs":
        start = source.index("/// The side a rule holds on (NeoForge's `DependencySide`).")
        end = source.index("impl Rule {", start)
        source = source[:start] + source[end:]
        source = source.replace("use mod_vocab::Side;", "pub use mod_vocab::Side;")
    source = source.replace("RuleSide", "Side")
    path.write_text(source)
