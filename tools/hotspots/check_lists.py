#!/usr/bin/env python3
"""A `pub use` under crates/horizon-mod-vocab/src/exports/ names at most 12 items in braces.

rustfmt re-wraps a brace list on every added name, so two PRs that each add a name to one long list always
conflict (tools/hotspots/README.md). Export a module's items with a glob, or put the new names in a fragment of
their own. A list's length is the number of names in the whole `pub use` (any visibility), nested braces included.
A fragment a switch removed (RETIRED) may not come back.

The lists that are longer today are in LONG_LISTS with their length on main as a ceiling: the check fails when a
listed file's longest list is longer than its entry, so shortening a list needs no change here. A file a planned
PR removes has no ceiling. The table is edited only to remove an entry, which the check asks for once the file has
no list over 12 names.

    python3 tools/hotspots/check_lists.py
"""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
EXPORTS = "crates/horizon-mod-vocab/src/exports"
LIMIT = 12

# file (under EXPORTS): (longest list allowed, or None while a planned PR removes the lists, why it stays).
# A list may be shorter than its entry, not longer. An entry whose file has no list over LIMIT any more fails the check
# until it is removed, so an exemption ends with its list.
LONG_LISTS = {
    "built.rs": (17, "no planned PR removes it"),
    "game.rs": (24, "no planned PR removes it"),
    # The Parts runtime and lookups of parts/shared.rs that the game reaches through `__private`.
    "parts_shared.rs": (15, "one parts file's game-side names"),
    "payload.rs": (17, "no planned PR removes it"),
    "payload_codec.rs": (13, "no planned PR removes it"),
    "phase.rs": (15, "no planned PR removes it"),
    "registry_keys.rs": (15, "rename list"),
    "structure_type.rs": (14, "no planned PR removes it"),
}

# Files a switch removed (file under EXPORTS: where its content lives now). One at the old path fails.
RETIRED = {
    "part_table.rs": "PR G of the hotspot design split it into one fragment per parts file: a name defined in "
                     "parts/<file>.rs is exported from exports/parts_<file>.rs, in the same scope",
}

USE = re.compile(r"\bpub(?:\s*\([^)]*\))?\s+use\s+([^;]*);")
COMMENT = re.compile(r"//[^\n]*|/\*.*?\*/", re.S)


def longest_list(source):
    """Names in the longest braced `pub use` of a Rust source (0 when none has braces)."""
    best = 0
    for m in USE.finditer(COMMENT.sub("", source)):
        tree = m.group(1)
        if "{" not in tree:
            continue
        names = [x for x in re.split(r"[{},]", tree) if x.strip() and not x.strip().endswith("::")]
        best = max(best, len(names))
    return best


def problems(root):
    found = []
    folder = Path(root) / EXPORTS
    for path in sorted(folder.rglob("*.rs")):
        name = path.relative_to(folder).as_posix()
        n = longest_list(path.read_text(encoding="utf-8"))
        entry = LONG_LISTS.get(name)
        shown = f"{EXPORTS}/{name}"
        if entry is None:
            if n > LIMIT:
                found.append(f"{shown}: a `pub use` names {n} items in braces, more than {LIMIT}. Two PRs that each "
                             "add a name to a long list conflict when rustfmt re-wraps it: export the module with a "
                             "glob, or put the new names in a `pub use` of their own.")
        elif n <= LIMIT:
            found.append(f"{shown}: no `pub use` list there names more than {LIMIT} items any more: remove its entry "
                         "from LONG_LISTS in tools/hotspots/check_lists.py.")
        elif entry[0] is not None and n > entry[0]:
            found.append(f"{shown}: its longest `pub use` list grew to {n} names (at most {entry[0]}, and it may only "
                         "shrink). Put the new names in a `pub use` of their own.")
    for name, now in sorted(RETIRED.items()):
        if (folder / name).exists():
            found.append(f"{EXPORTS}/{name} was removed: {now}.")
    for name in sorted(LONG_LISTS):
        if not (folder / name).exists():
            found.append(f"{EXPORTS}/{name} is gone: remove its entry from LONG_LISTS in tools/hotspots/check_lists.py.")
    return found


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--root", default=str(ROOT))
    args = ap.parse_args(argv)
    found = problems(args.root)
    for line in found:
        print(f"error: {line}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
