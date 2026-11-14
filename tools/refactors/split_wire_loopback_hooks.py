#!/usr/bin/env python3
"""Move the wire-loopback guest's hand-written `impl Hooks for Mod` into the files of the features it serves.

One-off (arismoko/horizon#4281). Reads tests/wire_loopback/interaction_fixture.rs, writes each hook method into
inherent `impl <Feature>Hooks` in the file that owns it (appending to a feature's existing file, or creating a
small file named after the hook topic), splits `register` by its statements, deletes interaction_fixture.rs and
asserts that the multiset of normalized method bodies is the same before and after. Prints the assignment table.

    python3 tools/refactors/split_wire_loopback_hooks.py
"""

from collections import Counter
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
FOLDER = ROOT / "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback"
OLD = FOLDER / "interaction_fixture.rs"

# method -> file stem. A stem that has a file already gets the impl appended; another is created.
OWNER = {
    "entity_interact": "use_item_hand",
    "datapack_sync": "datapack_sync_hook",
    "loot_function": "loot_function_hook",
    "loot_condition": "loot_query",
    "register_load_conditions": "load_conditions",
    "register_entity_renderers": "entity_renderers",
    "register_entity_layers": "entity_layers",
    "register_special_model_renderer": "special_model_renderers",
    "register_json_animation_types": "json_animation_types_hook",
    "subscribed": "render_gui_pre_hook",
    "listeners": "render_gui_pre_hook",
    "render_gui_pre": "render_gui_pre_hook",
    "item_tooltip": "item_tooltip_hook",
    "apply_effect_tick": "apply_effect_tick_hook",
    "render_living_pre": "render_living_hooks",
    "render_living_post": "render_living_hooks",
    "render_player_pre": "render_player_hooks",
    "render_player_post": "render_player_hooks",
    "item_picked_up_post": "item_picked_up_hooks",
    "item_picked_up_partial": "item_picked_up_hooks",
    "viewed_level_sound": "viewed_level_sound",
    "item_toss": "item_toss_hook",
    "game_rule_changed": "game_rule_changed_hook",
    "container_loot_filled": "container_loot_filled",
    "item_fished": "item_fished_hook",
    "loot_drops": "loot_drops_hook",
    "living_drops": "item_drop_state",
    "item_stacked_on_other": "item_stacked_on_other_hook",
    "living_jump_power": "living_jump_power_hook",
}
# `register` statements: a call into a feature's module goes to that feature; the rest is the shared registry.
REGISTER_SHARED = "fixture_registry"
MODULES = {path.stem for path in FOLDER.glob("*.rs")} | set(OWNER.values()) | {REGISTER_SHARED}


def struct_name(stem):
    topic = re.sub(r"_hooks?$", "", stem)
    return "".join(part.capitalize() for part in topic.split("_")) + "Hooks"


def methods(text):
    """(name, source lines) of every method of the one impl in `text`, the impl's own lines left out."""
    found, current = [], None
    for line in text.splitlines():
        if current is None:
            name = re.match(r"    (?:pub\(super\) )?fn (\w+)", line)
            if name:
                current = (name.group(1), [line])
                if line.rstrip().endswith("}") and line.count("{") == line.count("}"):
                    found.append(current)
                    current = None
        else:
            current[1].append(line)
            if line == "    }":
                found.append(current)
                current = None
    return found


def own_paths(source, stems):
    """`stem::item` inside the file of `stem` is `self::item` there."""
    return re.sub(rf"\b(?:{'|'.join(map(re.escape, stems))})::", "self::", source)


def normalize(source):
    """The method as one line, with the module paths that moving it changes (`stem::` or `self::`) dropped."""
    names = "|".join(map(re.escape, sorted(MODULES | {"self"})))
    return " ".join(re.sub(rf"\b(?:{names})::|pub\(super\) ", "", source).split())


def split_register(lines):
    """(stem, statement lines) of `register`'s body, statement by statement."""
    body = lines[1:-1]
    groups, statement = [], []
    for line in body:
        statement.append(line)
        if line.rstrip().endswith(";") and line.count("(") <= line.count(")") + 0 and not line.rstrip().endswith("(;"):
            groups.append(statement)
            statement = []
    assert not statement, statement
    out = []
    for group in groups:
        head = group[0].strip()
        call = re.match(r"(\w+)::(?:\w+)\(registrar\);", head)
        out.append((call.group(1) if call else REGISTER_SHARED, group))
    return out


def main():
    old_methods = methods(OLD.read_text())
    assert len(old_methods) == len(OWNER) + 1, (len(old_methods), len(OWNER))
    stems = set(OWNER.values()) | {REGISTER_SHARED}
    per_file = {}
    signature_of_register = None
    before = Counter()
    for name, lines in old_methods:
        if name == "register":
            signature_of_register = lines[0]
            for stem, group in split_register(lines):
                per_file.setdefault(stem, {}).setdefault("register", []).extend(group)
                before[normalize("register " + " ".join(group))] += 1
            continue
        source = "\n".join(lines)
        before[normalize(source)] += 1
        per_file.setdefault(OWNER[name], {})[name] = lines
    assert signature_of_register
    # register keeps the signature of the old method; its statements are the ones owned by the file.
    register_head = "    fn register(&self, registrar: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {"
    assert signature_of_register.strip() == register_head.strip(), signature_of_register
    after = Counter()
    table = []
    for stem in sorted(per_file):
        path = FOLDER / f"{stem}.rs"
        text = path.read_text() if path.exists() else "use super::*;\n"
        if not re.search(r"^use super::\*;", text, re.M):
            text = "use super::*;\n" + text
        struct = struct_name(stem)
        block = [f"\npub(super) struct {struct};", f"impl {struct} {{"]
        for name, lines in per_file[stem].items():
            if name == "register":
                lines = [register_head, *own_paths("\n".join(lines), [stem]).split("\n"), "    }"]
            else:
                lines = own_paths("\n".join(lines), [stem]).split("\n")
            lines = [re.sub(r"^    fn ", "    pub(super) fn ", lines[0])] + lines[1:]
            block += lines
            table.append((name, f"{stem}.rs"))
        block.append("}")
        path.write_text(text.rstrip("\n") + "\n" + "\n".join(block) + "\n")
    # Conservation: re-read the written impls and compare their normalized method bodies to the old ones.
    for stem in per_file:
        text = (FOLDER / f"{stem}.rs").read_text()
        impl = text[text.index(f"impl {struct_name(stem)} {{") :]
        for name, lines in methods(impl):
            if name == "register":
                for _, group in split_register(lines):
                    after[normalize("register " + " ".join(group))] += 1
            else:
                after[normalize("\n".join(lines))] += 1
    OLD.unlink()
    root = FOLDER.parent / "wire_loopback.rs"
    text = root.read_text()
    include = 'include!(concat!(env!("OUT_DIR"), "/wire_loopback_modules.rs"));\n'
    assert include in text and "Box::new(interaction_fixture::Mod)" in text
    text = text.replace(include, include + "// The guest's one Hooks object, assembled by build.rs from the inherent `impl <Feature>Hooks` of those files.\n"
                        + 'include!(concat!(env!("OUT_DIR"), "/wire_loopback_hooks.rs"));\n')
    root.write_text(text.replace("Box::new(interaction_fixture::Mod)", "Box::new(Mod)"))
    for name, stem in sorted(table, key=lambda row: row[1]):
        print(f"{name:34} -> {stem}")
    if before != after:
        print("CONSERVATION FAILED", (before - after), (after - before))
        return 1
    print(f"CONSERVATION: {sum(before.values())} method bodies and register statements, multiset equal before and after")
    return 0


if __name__ == "__main__":
    sys.exit(main())
