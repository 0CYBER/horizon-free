#!/usr/bin/env python3
"""Move GameLevel's LevelAccess implementation verbatim into the blocks area.

Run `python3 tools/extract_level_access_impl.py` from any directory. The existing
.game-area discovery registers the sibling automatically. This deterministic,
one-time refactor recognizes the exact top-level impl and its unindented closing
brace (method bodies are indented). It refuses to overwrite an existing output;
a repeated run after extraction is a no-op. Imports are a separate manual step.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/horizon-client/src/game.rs"
TARGET = SOURCE.parent / "game/blocks/level_access.rs"
MARKER = "impl LevelAccess for GameLevel<'_> {\n"


def main():
    source = SOURCE.read_text(encoding="utf-8")
    if MARKER not in source:
        if TARGET.is_file() and MARKER in TARGET.read_text(encoding="utf-8"):
            return
        raise SystemExit("LevelAccess implementation not found")
    if source.count(MARKER) != 1 or TARGET.exists():
        raise SystemExit("Ambiguous implementation or existing destination")
    start = source.index(MARKER)
    end = source.index("\n}\n", start) + len("\n}\n")
    block = source[start:end]
    TARGET.write_text(block, encoding="utf-8")
    SOURCE.write_text(source[:start] + source[end:], encoding="utf-8")


if __name__ == "__main__":
    main()
