"""Read native hook implementations across the game's area modules."""
from pathlib import Path


def native_hook_sources():
    directory = Path(__file__).resolve().parents[1] / "crates/horizon-client/src/game/native_hooks"
    return "\n".join(path.read_text(encoding="utf-8") for path in sorted(directory.glob("*.rs"))).replace(
        "pub(in crate::game) fn", "pub(super) fn"
    )
