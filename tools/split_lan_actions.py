#!/usr/bin/env python3
"""One-shot LAN action move; --verify BASE proves the exact move and payload multiset.

Declared edits: impl/use/include wrappers, equivalent method visibility one module
level deeper, game-relative paths in moved methods, and physical port-ledger paths.
Shared state, helpers and mixed tests retain their lexical scope and original text.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import sys

from split_native_hooks import ROOT, formatted, layout, payload, run

# Section boundaries only: whole methods are classified using syn's source spans.
SECTIONS = dict(zip(
    "as_guest guest_may_open under_spawn_protection guest_tick_death guest_command "
    "guest_respawn tick_guests guest_die wake_guest_from server_spin_attack "
    "lan_start_riding_jump guest_stop_digging guest_pickups guest_use_sounds".split(),
    "turns menus packets interaction commands respawn tick respawn sleep tick "
    "movement mining pickups effects".split(),
))
SHARED_DOC = "// Shared state, helpers and mixed tests stay at the original LAN action scope.\n"
POINT_DOC = "/// Kept at LAN action scope to preserve its derived internals point identity.\n"
INCLUDE = 'include!(concat!(env!("OUT_DIR"), "/lan_action_modules.rs"));\n'


def source_path(base=None):
    for path in ("crates/horizon-client/src/game/network/lan_actions.rs",
                 "crates/horizon-client/src/game/lan_actions.rs"):
        if (run("git", "ls-tree", base, "--", path).strip() if base
                else (ROOT / path).is_file()):
            return Path(path)
    raise ValueError("base has no unsplit LAN actions source")


def split(source, binary):
    lines = source.splitlines(keepends=True)
    manifest = layout(source, binary)
    game = [item for item in manifest["items"] if "methods" in item
            and lines[item["start"] - 1].startswith("impl Game<")]
    assert len(game) == 1, "expected one original Game impl"
    item = game[0]
    methods = item["methods"]
    first = next(i["start"] - 1 for i in manifest["items"]
                 if not lines[i["start"] - 1].startswith("use "))
    outputs = defaultdict(str)
    outputs["mod.rs"] = "".join(lines[:first]) + INCLUDE + 'include!("shared.rs");\n'
    outputs["shared.rs"] = SHARED_DOC + "".join(lines[first:item["start"] - 1])
    normalized = "".join(lines[first:item["start"]])
    cursor = item["start"]  # line after the impl header
    area = None
    ownership = {}
    for method in methods:
        area = SECTIONS.get(method["name"], area)
        assert area, "first method has no area"
        text = "".join(lines[cursor:method["end"]])
        cursor = method["end"]
        if method["name"] == "vehicle_has_other_viewers":
            outputs["shared.rs"] += POINT_DOC + "impl Game<'_> {\n" + text + "}\n"
            ownership[method["name"]] = "shared"
        else:
            text = text.replace("super::", "crate::game::")
            text = text.replace("pub(super)", "pub(in crate::game)")
            text = re.sub(r"(?m)^    fn ", "    pub(super) fn ", text)
            outputs[area + ".rs"] += text
            ownership[method["name"]] = area
        normalized += text
    normalized += "".join(lines[cursor:])
    outputs["shared.rs"] += "".join(lines[item["end"]:])
    for name in outputs:
        if name not in ("mod.rs", "shared.rs"):
            outputs[name] = "use super::*;\n\nimpl Game<'_> {\n" + outputs[name] + "}\n"
        outputs[name] = formatted(outputs[name])
    return dict(outputs), normalized, ownership


def ledger(source, old, directory, ownership):
    # Resolve methods individually; prose mentioning the old file is not a source pointer.
    return re.sub(re.escape(str(old)) + r"::(Game::)?(\w+)",
                  lambda m: str(directory / (ownership[m[2]] + ".rs")) + "::"
                  + (m[1] or "") + m[2] if m[2] in ownership else m[0], source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", metavar="BASE")
    parser.add_argument("--base", metavar="BASE")
    args = parser.parse_args()
    base = args.verify or args.base
    old = source_path(base)
    directory = old.with_suffix("")
    source = run("git", "show", f"{base}:{old}") if base else (ROOT / old).read_text()
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1"))
    run("cargo", "build", "-q", "-p", "horizon-mod-api-gen", "--bin", "native-hook-layout")
    binary = Path(metadata["target_directory"]) / "debug" / (
        "native-hook-layout.exe" if sys.platform == "win32" else "native-hook-layout")
    expected, normalized, ownership = split(source, binary)
    if args.verify:
        actual = {p.name: p.read_text() for p in (ROOT / directory).glob("*.rs")}
        assert not (ROOT / old).exists(), "old hotspot source still exists"
        assert actual == expected, "files differ from the deterministic formatted move"
        actual["shared.rs"] = actual["shared.rs"].replace(SHARED_DOC, "", 1).replace(POINT_DOC, "", 1)
        before = payload({"before.rs": formatted(normalized)}, binary)
        after = payload(actual, binary)
        assert before == after, f"payload mismatch: removed {before - after}; added {after - before}"
        print(f"Moved payload line multisets: equal ({sum(before.values())} nonblank lines)")
        print(f"Whole Game methods: {len(ownership)}; moved: {len(ownership) - 1}; areas: {len(set(ownership.values()) - {'shared'})}; shared point-identity exception: 1")
        print("Exact formatted output and within-area item order: identical to scripted move")
        print("Declared edits: impl/use/include wrappers; equivalent visibility; game-relative paths; ledger paths")
    else:
        (ROOT / directory).mkdir(exist_ok=True)
        for name, text in expected.items():
            (ROOT / directory / name).write_text(text)
        (ROOT / old).unlink(missing_ok=True)
    for path in sorted((ROOT / "tools/port_map").glob("*.tsv")):
        original = run("git", "show", f"{base}:{path.relative_to(ROOT)}") if base else path.read_text()
        updated = ledger(original, old, directory, ownership)
        if args.verify:
            assert path.read_text() == updated, f"unexpected ledger edit: {path.name}"
        elif updated != original:
            path.write_text(updated)


if __name__ == "__main__":
    main()
