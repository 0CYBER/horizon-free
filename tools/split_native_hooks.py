#!/usr/bin/env python3
"""One-shot B2 move, or reproduce and verify it against the B1 git revision.

The Rust generator owns both catalog parsing and syn item ranges. Declared edits:
new impl/import/include wrappers; effective visibility preserved one module deeper;
game-relative paths made absolute; source-reading tests pointed at their new files;
catalog target paths gain the owning area. No method body is otherwise rewritten.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = Path("crates/horizon-client/src/game/native_hooks")
CATALOG = Path("crates/horizon-mod-vocab/src/hooks")


def run(*args, source=None):
    return subprocess.check_output(args, cwd=ROOT, input=source, text=True)


def layout(source, binary):
    return json.loads(run(str(binary), source=source))


def formatted(source):
    return run("rustfmt", "--edition", "2024", "--config", "skip_children=true",
               "--emit", "stdout", source=source)


def split(source, manifest):
    lines = source.splitlines(keepends=True)
    areas = manifest["areas"]
    outputs = defaultdict(str)
    # Keep the original module docs, imports and reexports in their lexical scope.
    first = next(i["start"] - 1 for i in manifest["items"]
                 if not lines[i["start"] - 1].startswith(("use ", "pub use ", "pub(crate) use ", "mod fire;")))
    outputs["mod"] = "".join(lines[:first]).replace("mod fire;", "/// Generated dispatch entry points, not hand-written hook implementations.\nmod fire;")
    outputs["mod"] += 'include!(concat!(env!("OUT_DIR"), "/native_hook_modules.rs"));\ninclude!("shared.rs");\n'
    cursor = first
    for item in manifest["items"]:
        if item["end"] <= first:
            continue
        start, end = item["start"] - 1, item["end"]
        prefix = "".join(lines[cursor:start])
        cursor = end
        if "methods" not in item:
            text = prefix + "".join(lines[start:end])
            test = re.search(r"mod (\w+)_tests \{", text)
            refs = {areas[n] for n in re.findall(r"\bnative_\w+\b", text) if n in areas}
            area = areas.get("native_" + test[1]) if test else None
            if test and not area and len(refs) == 1:
                area = refs.pop()
            if test and "pub(super) mod " in text:
                area = None  # Shared fixture called by other game test modules.
            if area:
                # Tests' parent becomes the area; game references keep their target.
                text = text.replace("super::super::", "crate::game::")
                outputs[area] += text
            else:
                outputs["shared"] += text
            continue
        methods = item["methods"]
        header_end = methods[0]["start"] - 1
        # Ordinary non-doc comments between the impl and first method go with it.
        while not lines[header_end - 1].rstrip().endswith("{"):
            header_end -= 1
        header = "".join(lines[start:header_end])
        groups = defaultdict(str)
        previous = header_end
        for method in methods:
            text = "".join(lines[previous:method["end"]])
            previous = method["end"]
            area = areas.get(method["name"], "shared")
            if area != "shared":
                text = text.replace("pub(super)", "pub(in crate::game)")
                text = text.replace("super::", "crate::game::")
            groups[area] += text
        tail = "".join(lines[previous:end - 1])
        for area, body in groups.items():
            outputs[area] += prefix + header + body + tail + "}\n"
    outputs["shared"] += "".join(lines[cursor:])
    # Source-reading tests are the only non-path test edits; their assertions stay intact.
    shared = outputs["shared"]
    shared = shared.replace('include_str!("mod.rs").replace', 'include_str!("base.rs").replace')
    shared = shared.replace('"    pub(super) fn native_set_block_property', '"    pub(in crate::game) fn native_set_block_property')
    old = '''        let source = include_str!("mod.rs");
        let code = source
            .split("\\nmod tests {")
            .next()
            .unwrap()
            .trim_end()
            .strip_suffix("#[cfg(test)]")
            .expect("the tests module has its cfg");
        let lines = code.lines().filter(|l| !l.trim_start().starts_with("//"));'''
    new = '''        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/game/native_hooks");
        let mut code = String::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "rs")
                && !matches!(path.file_stem().unwrap().to_str().unwrap(), "mod" | "fire")
            {
                let source = std::fs::read_to_string(path).unwrap();
                code.push_str(source.split("#[cfg(test)]").next().unwrap());
            }
        }
        let lines = code.lines().filter(|l| !l.trim_start().starts_with("//"));'''
    assert old in shared, "source-reading test changed: review the declared edit"
    outputs["shared"] = shared.replace(old, new)
    return {name + ".rs": formatted(("use super::*;\n\n" if name not in ("mod", "shared") else "") + text)
            for name, text in sorted(outputs.items())}


def payload(files, binary):
    """Exclude only added module/use/impl wrappers, using syn's own line ranges."""
    result = Counter()
    for name, source in files.items():
        if name == "mod.rs":
            continue  # Original docs/imports are checked byte-for-byte separately below.
        lines = source.splitlines()
        excluded = set()
        for item in layout(source, binary)["items"]:
            if "methods" in item:
                first = item["methods"][0]["start"] - 1
                excluded.update(range(item["start"] - 1, first))
                excluded.add(item["end"] - 1)
        result.update(line.strip() for n, line in enumerate(lines)
                      if n not in excluded and line.strip() and line != "use super::*;")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", metavar="B1_REVISION")
    parser.add_argument("--base", metavar="B1_REVISION", help="reproduce the split from git")
    args = parser.parse_args()
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1"))
    run("cargo", "build", "-q", "-p", "horizon-mod-api-gen", "--bin", "native-hook-layout")
    executable = "native-hook-layout.exe" if sys.platform == "win32" else "native-hook-layout"
    binary = Path(metadata["target_directory"]) / "debug" / executable
    path = DIRECTORY / "mod.rs"
    base = args.verify or args.base
    source = run("git", "show", f"{base}:{path}") if base else (ROOT / path).read_text()
    manifest = layout(source, binary)
    expected = split(source, manifest)
    if args.verify:
        actual = {p.name: p.read_text() for p in (ROOT / DIRECTORY).glob("*.rs") if p.name != "fire.rs"}
        assert actual.keys() == expected.keys(), "unexpected or missing area files"
        assert actual["mod.rs"] == expected["mod.rs"], "module imports/wrappers differ"
        before, after = payload(expected, binary), payload(actual, binary)
        assert before == after, f"payload lines differ: removed {before - after}; added {after - before}"
        assert actual == expected, "item contents/order differ from the deterministic move"
        moved = sum(method["name"] in manifest["areas"] for item in manifest["items"]
                    for method in item.get("methods", []))
        print(f"Catalog mapping: {len(manifest['areas'])} target names; {moved} methods; {len(expected) - 2} area files")
        print(f"Moved payload line multisets: equal ({sum(before.values())} nonblank lines)")
        print("Declared edits: impl/use/include wrappers; preserved visibility; game-relative paths; source-reading tests; catalog target module paths")
        print("Exact formatted output and within-area item order: identical to scripted move")
        for p in (ROOT / CATALOG).glob("*.rs"):
            original = run("git", "show", f"{args.verify}:{p.relative_to(ROOT)}")
            assert p.read_text() == retarget(original, manifest["areas"]), f"unexpected catalog edit: {p.name}"
        print("Catalog target edits: exact; no dispatch contracts changed")
    else:
        for name, text in expected.items():
            (ROOT / DIRECTORY / name).write_text(text)
        for p in (ROOT / CATALOG).glob("*.rs"):
            p.write_text(retarget(p.read_text(), manifest["areas"]))
        print(f"Split {len(manifest['areas'])} catalog target names into {len(expected) - 2} areas")


def retarget(source, areas):
    source = re.sub(r"horizon_client::game::native_hooks::(Game(?:Level)?|HitLevel)(<'_>)?::(native_\w+)",
                    lambda m: f"horizon_client::game::native_hooks::{areas[m[3]]}::{m[1]}{m[2] or ''}::{m[3]}", source)
    return source.replace("in `native_hooks.rs`", "in `native_hooks/<area>.rs`")


if __name__ == "__main__":
    main()
