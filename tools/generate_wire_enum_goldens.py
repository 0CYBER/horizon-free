#!/usr/bin/env python3
"""Generate/check actual-type Wire layout regression tests from compiler expansion.

After a deliberate interface change, capture hashes with BEFORE semantics first;
never record the implementation under test as its own expected answer. The input
manifest is machine-generated from actual baseline symbols, not a type registry.
See docs/engineering/wire-enum-layouts.md for the reproducible commands/provenance.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
GOLDENS = ROOT / "tools/fixtures/wire_enum_layouts.json"
OUTPUT = ROOT / "crates/horizon-mod-vocab/src/wire/fieldless_enum_goldens.rs"
# Rust literals whose contents are not syntax: raw strings, strings (an escape may be a
# line continuation) and char literals, so a quote or brace inside one ('"', '{') stays inside.
LITERAL = (r'r(?P<hashes>#*)"[\s\S]*?"(?P=hashes)|"(?:\\[\s\S]|[^"\\])*"'
           r"|'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|[\s\S])|[^'\\\n])'")


def discover(source, required=None):
    """Find unit-variant enums with Wire impls; no dependence on layout body."""
    # rustc retains doc comments in expansion; their punctuation is not syntax.
    source = re.sub(LITERAL + r'|//[^\n]*|/\*[\s\S]*?\*/',
                    lambda match: "" if match[0].startswith(("//", "/*")) else match[0], source)
    token_matches = list(re.finditer(LITERAL + r'|[A-Za-z_][A-Za-z_0-9]*|::|[^\s]', source))
    tokens = [match[0] for match in token_matches]
    stack, pairs = [], {}
    for i, token in enumerate(tokens):
        if token == "{":
            stack.append(i)
        elif token == "}":
            pairs[stack.pop()] = i
    modules = [(i + 2, pairs[i + 2], tokens[i + 1])
               for i in range(len(tokens) - 2)
               if tokens[i] == "mod" and tokens[i + 2] == "{"]

    def identity(position, name):
        if name.startswith("crate::"):
            return name
        parents = [name for start, end, name in modules
                   if token_matches[start].start() < position < token_matches[end].end()]
        return "::".join(["crate", *parents, name])
    headers = re.sub(LITERAL, lambda match: ' ' * len(match[0]), source)
    supported = r"impl\s+(?:::)?[\w:]*wire::Wire\s+for\s+(\w+)\s*\{"
    derived_headers = [
        (match[2], identity(match.start(), match[3]))
        for match in re.finditer(
            r"((?:#\[[^\]]*\]\s*)+)(impl\b[^{};]*\bWire\b[^{};]*\bfor\s+([\w:]+)[^{};]*\{)", headers)
        if re.search(r"#\[automatically_derived\]", match[1])
    ]
    impls = {path for header, path in derived_headers
             if (match := re.fullmatch(supported, header)) is not None}
    local_units = {}
    for i, token in enumerate(tokens[:-2]):
        if token != "enum":
            continue
        name = tokens[i + 1]
        start = i + 2
        while tokens[start] != "{":
            start += 1
        depth, end = 1, start + 1
        while depth:
            depth += (tokens[end] == "{") - (tokens[end] == "}")
            end += 1
        body = tokens[start + 1:end - 1]
        # Variant attributes (doc/allow/default/...) are not payload fields.
        clean = []
        cursor = 0
        while cursor < len(body):
            if body[cursor:cursor + 2] == ["#", "["]:
                cursor += 2
                brackets = 1
                while brackets:
                    brackets += (body[cursor] == "[") - (body[cursor] == "]")
                    cursor += 1
            else:
                clean.append(body[cursor])
                cursor += 1
        body = clean
        # Before '=' a '(' or '{' denotes a payload, not a discriminant.
        unit, discriminant = True, False
        for item in body:
            if item == "=":
                discriminant = True
            elif item == ",":
                discriminant = False
            elif item in ("(", "{") and not discriminant:
                unit = False
        if unit:
            path = identity(token_matches[i].start(), name)
            local_units[path] = local_units.get(path, 0) + 1
    # This deliberately supports the derive's simple owned impl shape only.
    # Refuse unusual headers targeting local unit enums instead of silently
    # dropping them from coverage (including const-generic impl parameters).
    for header, name in derived_headers:
        if name in local_units and (required is None or name in required) and re.fullmatch(supported, header) is None:
            raise ValueError(f"unsupported Wire impl header for local fieldless enum: {name}")
    enums = set(local_units) & impls
    for name in enums:
        if local_units[name] != 1:
            raise ValueError(f"ambiguous enum leaf identity: {name}")
    return enums


def validate(manifest, required, source):
    expected = {entry["canonical_path"] for entry in manifest["enums"]}
    if len(expected) != len(manifest["enums"]) or expected != set(required):
        raise ValueError("missing or duplicate frozen baseline pin")
    retired = set(manifest.get("retired_identities", []))
    actual = discover(source, expected | retired)
    if not expected <= actual:
        raise ValueError(
            "Frozen enum removed or no longer derives fieldless Wire: "
            f"{sorted(expected - actual)}. Remove its matching pins from "
            "wire_enum_layouts.json and wire_enum_baseline_identities.json in the "
            "SAME PR as the source change, and record its identity in retired_identities. "
            "Removal alone is not a layout hash regression.")
    live_retired = retired & actual
    if retired & expected or live_retired:
        raise ValueError(f"cannot retire a live fieldless Wire enum pin: {sorted((retired & expected) | live_retired)}")
    return expected


def render(manifest):
    lines = ["// @generated by tools/generate_wire_enum_goldens.py; do not edit.",
             "use super::Wire;", "", "#[test]",
             "fn existing_fieldless_enum_layouts_match_baseline() {"]
    for entry in manifest["enums"]:
        expected = "super::hook_id_runtime_reference()" if entry.get("comparison") == "same_source_generated_variants" else f'0x{entry["hash"]}'
        lines.append(f'    assert_eq!(<{entry["path"]} as Wire>::layout(&mut vec![]), {expected}, "{entry["name"]}");')
    lines.append("}\n")
    result = subprocess.run(["rustfmt", "--edition", "2024", "--emit", "stdout"],
                            input="\n".join(lines), text=True, capture_output=True, check=True)
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expanded", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    manifest = json.loads(GOLDENS.read_text())
    required = json.loads((ROOT / "tools/fixtures/wire_enum_baseline_identities.json").read_text())
    actual = validate(manifest, required, args.expanded.read_text())
    rendered = render(manifest)
    if args.check:
        if OUTPUT.read_text() != rendered:
            raise ValueError("generated golden tests are stale")
    else:
        OUTPUT.write_text(rendered)
    print(f"Wire enum goldens: coverage of {len(actual)} frozen-baseline fieldless derives")


if __name__ == "__main__":
    main()
