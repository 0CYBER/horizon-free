#!/usr/bin/env python3
"""One-shot fixture move, reproduced with --verify BASE.

Declared edits: remove the tests/stubs/API scope envelopes; add generated includes
at those same lexical positions; repeat stubs/API envelopes per area; add the two
exception comments; keep the macro alias import in the shell; two declared
rustfmt-only line reflows. No tokens in item
bodies, signatures, visibility or assertions change.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path("crates/horizon-mod-api-gen/dispatch-tests/src/lib.rs")
DIRECTORY = SOURCE.parent / "fixtures"
SHELL = '#[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    mod stubs {\n        include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_stubs.rs"));\n    }\n    use generated as mod_api;\n    include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_root.rs"));\n}\n'
REFLOWS = [
    ('let subscribers = horizon_client_core::subscribers::Subscribers::new(\n            HOOK_COUNT,\n            mods.len(),\n            subscriptions,\n        );',
     'let subscribers =\n    horizon_client_core::subscribers::Subscribers::new(HOOK_COUNT, mods.len(), subscriptions);'),
    ("fn item_model_property(\n                &self,\n                _: &dyn ClientHost,\n                event: &mut ItemModelPropertyEvent<'_>,\n            ) {",
     "fn item_model_property(&self, _: &dyn ClientHost, event: &mut ItemModelPropertyEvent<'_>) {"),
]
COMMENTS = {
    "shared": "//! Shared exception: dispatch infrastructure and cross-area fixture types/tests; no new hooks.\n",
    "base": "//! Base exception: legacy base-catalog fixtures; new hooks use their own area.\n",
}


def run(*args, source=None):
    return subprocess.check_output(args, cwd=ROOT, input=source, text=True)


def formatted(source):
    return run("rustfmt", "--edition", "2024", "--config", "skip_children=true",
               "--emit", "stdout", source=source)


def include(scope):
    return f'include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_{scope}.rs"));\n'


def split(source, manifest, references):
    lines = source.splitlines(keepends=True)
    groups = defaultdict(lambda: defaultdict(str))
    excluded = set()
    test_module = manifest["items"][0]
    excluded.update([0, 1, test_module["end"] - 1])

    def chunks(items, start):
        for item in items:
            yield item, "".join(lines[start:item["end"]])
            start = item["end"]

    def types(items, start, scope):
        for item, text in chunks(items, start):
            owners = {area for area, words in references.items() if item["name"] in words}
            area = next(iter(owners)) if len(owners) == 1 else "shared"
            groups[area][scope] += text

    for item, text in chunks(test_module["children"], 2):
        start, end = item["start"] - 1, item["end"]
        if item["name"] == "stubs":
            excluded.update([start, start + 1, end - 1])
            types(item["children"], start + 2, "stubs")
        elif item["name"] == "generated":
            api = next(i for i in item["children"] if i["name"] == "mod_api")
            a, b = api["start"] - 1, api["end"]
            excluded.update([a, b - 1])
            text = text.replace("".join(lines[a + 1:b - 1]), include("api"))
            groups["shared"]["root"] += text
            types(api["children"], a + 1, "api")
        elif text.strip() == "use generated as mod_api;":
            excluded.add(start)  # Keep this explicit macro alias in the shell for Rust name resolution.
        else:
            owners = {manifest["hooks"][name] for name in re.findall(r"\bdispatch_(\w+)", text)
                      if name in manifest["hooks"]}
            area = next(iter(owners)) if text.lstrip().startswith("#[test]") and len(owners) == 1 else "shared"
            groups[area]["root"] += text
    expected = {}
    for area, scopes in groups.items():
        text = COMMENTS.get(area, "") + scopes["root"]
        for scope in ["stubs", "api"]:
            if scopes[scope]:
                text += f"\nmod {scope} {{\n" + scopes[scope] + "}\n"
        expected[area + ".rs"] = formatted(text.lstrip("\n"))
    original = "".join(line for n, line in enumerate(lines) if n not in excluded)
    for old, new in REFLOWS:
        assert old in original, "declared rustfmt reflow changed"
        original = original.replace(old, new)
    before = Counter(line.strip() for line in original.splitlines() if line.strip())
    return expected, before


def payload(files):
    lines = []
    for text in files.values():
        for line in text.splitlines():
            value = line.strip()
            if not value or value.startswith(("//! Shared exception:", "//! Base exception:")):
                continue
            lines.append(value)
    # Remove only the newly repeated envelope open/close lines, not body braces.
    for text in files.values():
        for scope in ["stubs", "api"]:
            if f"\nmod {scope} {{\n" in "\n" + text:
                lines.remove(f"mod {scope} {{")
                lines.remove("}")
    # The original API envelope still appears in shared's generated-module shell.
    lines.remove("pub(super) mod mod_api {")
    lines.remove("}")
    lines.remove(include("api").strip())
    return Counter(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", metavar="BASE")
    parser.add_argument("--base", required=False)
    args = parser.parse_args()
    base = args.verify or args.base
    source = run("git", "show", f"{base}:{SOURCE}") if base else (ROOT / SOURCE).read_text()
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1"))
    run("cargo", "build", "-q", "-p", "horizon-mod-api-gen", "--bin", "dispatch-fixture-layout")
    executable = "dispatch-fixture-layout.exe" if sys.platform == "win32" else "dispatch-fixture-layout"
    binary = Path(metadata["target_directory"]) / "debug" / executable
    manifest = json.loads(run(str(binary), source=source))
    native = "crates/horizon-client/src/game/native_hooks"
    paths = run("git", "ls-tree", "-r", "--name-only", base, native).splitlines() if base else [str(p.relative_to(ROOT)) for p in (ROOT / native).glob("*.rs")]
    references = {Path(p).stem: set(re.findall(r"\b\w+\b", run("git", "show", f"{base}:{p}") if base else (ROOT / p).read_text()))
                  for p in paths if p.endswith(".rs") and Path(p).stem not in ("mod", "shared", "fire")}
    expected, before = split(source, manifest, references)
    after = payload(expected)
    assert before == after, f"payload mismatch: removed {before - after}; added {after - before}"
    if args.verify:
        actual = {p.name: p.read_text() for p in (ROOT / DIRECTORY).glob("*.rs")}
        assert actual == expected, "fixture output differs from deterministic scripted move"
        assert (ROOT / SOURCE).read_text() == SHELL, "root fixture items have drifted"
        print(f"Moved-line multisets: equal ({sum(before.values())} nonblank lines)")
        print(f"Exact formatted output: {len(expected)} fragments; original item bodies and assertions unchanged")
        print(__doc__)
    else:
        (ROOT / DIRECTORY).mkdir(exist_ok=True)
        for name, text in expected.items():
            (ROOT / DIRECTORY / name).write_text(text)
        (ROOT / SOURCE).write_text(SHELL)
        print("\n".join(f"{name}: {len(text.splitlines())}" for name, text in sorted(expected.items())))


if __name__ == "__main__":
    main()
