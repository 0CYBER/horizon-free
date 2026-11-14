#!/usr/bin/env python3
"""Distinguish code link owners from content-only dispatch dependencies."""
from pathlib import Path
import re

paths = [
    "crates/horizon-mod-package/src/dependencies.rs",
    "crates/horizon-mod-wasm/src/lib.rs",
    "crates/horizon-mod-host/src/loading.rs",
    "crates/horizon-client/src/game/modding/mods/mod.rs",
    "launcher/src/native_mods.rs",
    "launcher/src/native_mods/internals_sync.rs",
    "launcher/src/native_mods/registry_compile.rs",
]
for filename in paths:
    path = Path(filename)
    source = path.read_text()
    # These files' has_api initializers belong to dependency Mod values.
    # False remains the content-only fixture default. Runtime mixed inputs
    # are corrected by hand from their actual package kind after this pass.
    source = re.sub(
        r"(?m)^( +)has_api: (true|false),$",
        lambda match: match[0] + "\n" + match[1] + "code: " + match[2] + ",",
        source,
    )
    path.write_text(source)
