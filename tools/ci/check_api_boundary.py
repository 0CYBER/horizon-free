#!/usr/bin/env python3
"""Guard the hand-written mod API boundary without building engine crates.

This is a conservative token-based source audit, not a Rust type checker. It scans
all source files (including generator export fragments and cfg-gated code), follows
use aliases, and checks public declarations and catalog signatures. Cargo then
proves type correctness. Private implementation bodies are not public signatures.
The only engine-owned vocabulary permitted is the explicit horizon-core leaf list
below. Additions require a reason, not a wildcard or a whole-file exemption.
"""
from __future__ import annotations

import argparse
import os
from dataclasses import dataclass
from pathlib import Path
import re
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
ENGINE = {
    "horizon_world", "horizon_client", "horizon_client_lib",
    "horizon_client_core", "horizon_client_render", "horizon_items",
    "horizon_assets", "horizon_mod_package", "horizon_mods", "horizon_core",
}
# Shared Minecraft vocabulary: plain, engine-independent values, parity RNG,
# and the JSON/NBT format converter. Never allow all of horizon-core.
LEAVES = {
    "arm_pose::ArmPose", "game_mode::GameMode", "geometry::Direction",
    "position::BlockPos", "position::ChunkPos", "random::LegacyRandom",
    "random::RandomSource", "bounding_box::BoundingBox",
    "equipment_slot::EquipmentSlot", "identifier::Identifier", "nbt::Compound",
    "nbt::Tag", "use_animation::UseAnimation", "connection_kind::ConnectionKind",
    "dimension::Dimension", "neighbour_update::NeighbourUpdate",
}
# Owner-approved JSON source-file parser exceptions, not arbitrary JSON carriers.
JSON_FUNCTIONS = {"fluid_capability.rs": {"from_json"}}



@dataclass(frozen=True)
class Token:
    value: str
    line: int


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    rule: str
    detail: str

    def __str__(self):
        return f"{self.path}:{self.line}: {self.rule}: {self.detail}"


def tokens(source: str) -> list[Token]:
    """Drop comments/strings (including nested comments and raw strings)."""
    result = []
    i, line = 0, 1
    pattern = re.compile(r"[A-Za-z_][A-Za-z_0-9]*|::|->|[^\s]")
    raw_string = re.compile(r'(?:b?r)(#*)"')
    character = re.compile(r"'(?:\\.|[^'\\])'")
    while i < len(source):
        start = i
        if source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
        elif source.startswith("/*", i):
            depth, i = 1, i + 2
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif source.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif match := raw_string.match(source, i):
            end = source.find('"' + match[1], match.end())
            i = len(source) if end < 0 else end + 1 + len(match[1])
        elif source[i] == '"' or (source[i] == "'" and character.match(source, i)):
            quote, i = source[i], i + 1
            while i < len(source):
                if source[i] == "\\":
                    i += 2
                elif source[i] == quote:
                    i += 1
                    break
                else:
                    i += 1
        elif source[i].isspace():
            i += 1
        else:
            match = pattern.match(source, i)
            result.append(Token(match[0], line))
            i = match.end()
        line += source[start:i].count("\n")
    return result


def pairs(ts):
    stack, result = [], {}
    for i, t in enumerate(ts):
        if t.value in {"(", "[", "{"}:
            stack.append(i)
        elif t.value in {")", "]", "}"} and stack:
            opening = stack.pop()
            result[opening] = i
    return result


def use_paths(values, prefix=()):
    """Expand nested use trees before aliases are discarded, including globs."""
    parts, start, depth = [], 0, 0
    for i, value in enumerate(values + [","]):
        if value == "{":
            depth += 1
        elif value == "}":
            depth -= 1
        elif value == "," and depth == 0:
            parts.append(values[start:i])
            start = i + 1
    for part in parts:
        if not part:
            continue
        if "{" in part:
            at = part.index("{")
            head = tuple(v for v in part[:at] if v != "::")
            yield from use_paths(part[at + 1:-1], prefix + head)
        else:
            at = part.index("as") if "as" in part else len(part)
            path = prefix + tuple(v for v in part[:at] if v != "::")
            alias = part[at + 1] if at < len(part) else path[-1]
            yield path, alias


def source_findings(path: Path, source: str):
    ts = tokens(source)
    ps = pairs(ts)
    def imports(start, end):
        # Imports are scope-wide, but imports in child/test scopes must not
        # overwrite the meaning of a name in a public parent signature.
        aliases = {}
        i = start
        while i < end:
            if ts[i].value == "use":
                stop = next(j for j in range(i + 1, end) if ts[j].value == ";")
                for imported, alias in use_paths([t.value for t in ts[i + 1:stop]]):
                    aliases[alias] = imported
                i = stop + 1
            elif ts[i].value in {"{", "(", "["}:
                i = ps[i] + 1
            else:
                i += 1
        return aliases

    def resolve(parts, aliases):
        seen = set()
        while parts and parts[0] in aliases and parts[0] not in seen:
            seen.add(parts[0])
            expanded = aliases[parts[0]]
            if expanded == (parts[0],):
                break
            parts = expanded + parts[1:]
        return parts

    findings = []

    def inspect(segment, aliases, function="", owner=""):
        for i, token in enumerate(segment):
            if i and segment[i - 1].value == "::":
                continue
            parts = [token.value]
            j = i + 1
            while j + 1 < len(segment) and segment[j].value == "::":
                parts.append(segment[j + 1].value)
                j += 2
            parts = resolve(tuple(parts), aliases)
            if not parts:
                continue
            if parts[0] in ENGINE:
                leaf = "::".join(parts[1:])
                if parts[0] != "horizon_core" or leaf not in LEAVES:
                    findings.append(Finding(str(path), token.line, "engine-type", "::".join(parts)))
            if parts[0] == "serde_json":
                allowed = path.name == "serde_json.rs" and path.parent.name == "exports"
                allowed |= function in JSON_FUNCTIONS.get(path.name, set())
                # Owner-approved: the recipe's data-pack file, read by a mod at reload.
                # Name the exact field/constructor, never the whole crafting module.
                if path.name == "crafting.rs" and owner == "Recipe":
                    field = next((segment[k - 1].value for k in range(i - 1, 0, -1)
                                  if segment[k].value == ":"), "")
                    allowed |= function == "new" or (not function and field == "json")
                if not allowed:
                    findings.append(Finding(str(path), token.line, "json-signature", "::".join(parts)))

    def walk(start, end, trait=False, private=False, inherited=None, owner=""):
        aliases = dict(inherited or {})
        aliases.update(imports(start, end))
        i = start
        while i < end:
            begin = i
            # Attributes do not hide engine types outside __private.
            test_only = False
            while i + 1 < end and ts[i].value == "#" and ts[i + 1].value == "[":
                attribute = [t.value for t in ts[i + 2:ps[i + 1]]]
                test_only |= attribute == ["cfg", "(", "test", ")"]
                i = ps[i + 1] + 1
            public = trait or (i < end and ts[i].value == "pub")
            if i < end and ts[i].value == "pub":
                i += 1
                if i < end and ts[i].value == "(":
                    public = False
                    i = ps[i] + 1
            # Skip signature modifiers.
            while i < end and ts[i].value in {"unsafe", "async", "default"}:
                i += 1
            if i >= end:
                break
            kind = ts[i].value
            name = ts[i + 1].value if i + 1 < end else ""
            j = i + 1
            while j < end and ts[j].value not in {";", "{"}:
                if ts[j].value in {"(", "["}:
                    j = ps[j] + 1
                else:
                    j += 1
            if j >= end:
                break
            if kind == "use":
                j = next(k for k in range(i, end) if ts[k].value == ";")
            body_end = ps[j] if ts[j].value == "{" else j
            in_private = private or test_only or (kind == "mod" and name == "__private")
            if not in_private:
                if kind == "use" and public:
                    for imported, _ in use_paths([t.value for t in ts[i + 1:j]]):
                        if "__private" in resolve(imported, aliases):
                            findings.append(Finding(str(path), ts[i].line, "private-export", "::".join(imported)))
                        inspect([Token(v, ts[i].line) for v in _join_path(imported)], aliases)
                elif public or kind == "type" or (kind == "fn" and path.parent.name == "hooks"):
                    inspect(ts[i:j], aliases, name if kind == "fn" else "", owner=owner)
                    if kind in {"struct", "enum", "union"} and body_end > j:
                        # Includes public record fields and enum payloads. Conservative
                        # for private fields: an engine-backed API wrapper is forbidden.
                        inspect(ts[j + 1:body_end], aliases, owner=name)
                if body_end > j and (kind in {"mod", "impl", "trait"} or path.parent.name == "hooks"):
                    walk(j + 1, body_end, trait=kind == "trait", private=in_private,
                         inherited=aliases, owner=name if kind == "impl" else owner)
            i = body_end + 1
            if i <= begin:
                raise AssertionError("scanner made no progress")

    walk(0, len(ts), private=(path.name == "__private.rs" and path.parent.name == "src")
         or "__private" in path.parts)
    return sorted(set(findings), key=lambda f: (f.path, f.line, f.rule, f.detail))


def _join_path(parts):
    return [v for i, part in enumerate(parts) for v in (("::", part) if i else (part,))]


def check(crate: Path):
    findings = []
    manifest_path = crate / "Cargo.toml"
    manifest = tomllib.loads(manifest_path.read_text())

    def dependencies(table):
        for section, value in table.items():
            if section in {"dependencies", "dev-dependencies", "build-dependencies"}:
                for alias, spec in value.items():
                    package = spec.get("package", alias) if isinstance(spec, dict) else alias
                    normalized = package.replace("-", "_")
                    if normalized in ENGINE and normalized != "horizon_core":
                        findings.append(Finding(str(manifest_path), 1, "engine-dependency", package))
            elif isinstance(value, dict):
                dependencies(value)

    dependencies(manifest)
    for path in sorted((crate / "src").rglob("*.rs")):
        if not path.name.endswith("_tests.rs"):
            findings.extend(source_findings(path, path.read_text()))
    return findings


def generated_findings(target: Path):
    """Scan the freshest vocabulary build output, never a fixture crate or stale profile.

    Source placeholders such as fn blocks() get their actual event field types
    from generation. Checking only the catalog would miss those public types.
    Cargo must run first; callers may not silently skip missing build output.
    """
    candidates = [p for p in target.rglob("mod_api_generated.rs")
                  if re.fullmatch(r"horizon-mod-vocab-[0-9a-f]+", p.parent.parent.name)]
    if not candidates:
        return [Finding(str(target), 1, "missing-generated", "build horizon-mod-vocab before the full boundary audit")]
    newest = max(candidates, key=lambda p: p.stat().st_mtime_ns)
    findings = []
    for name in ("mod_api_generated.rs", "mod_api_generated_host.rs"):
        path = newest.parent / name
        if not path.exists():
            findings.append(Finding(str(path), 1, "missing-generated", name))
        else:
            findings.extend(source_findings(path, path.read_text()))
    return findings


NEGATIVE = '''
use horizon_items::items::ItemStack as Borrowed;
pub use horizon_world::fluid::FluidState as Innocent;
pub use horizon_client_core::text::*;
pub fn stack() -> Borrowed { todo!() }
pub fn json() -> serde_json::Value { todo!() }
pub mod __private { pub use horizon_world::fluid::FluidState; }
pub use crate::__private::FluidState as Public;
'''


def negative_control():
    """Every invocation proves each rule is live, without mutating the checkout."""
    with tempfile.TemporaryDirectory() as directory:
        crate = Path(directory)
        (crate / "src/exports").mkdir(parents=True)
        (crate / "Cargo.toml").write_text('[dependencies]\nrenamed = { package = "horizon-world", version = "1" }\n')
        (crate / "src/exports/probe.rs").write_text(NEGATIVE)
        found = check(crate)
        required = {"engine-dependency", "engine-type", "json-signature", "private-export"}
        return required <= {f.rule for f in found} and sum(f.rule == "engine-type" for f in found) == 3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crate", type=Path, default=ROOT / "crates/horizon-mod-api")
    parser.add_argument("--source-only", action="store_true", help="early pre-build dependency/source gate only")
    parser.add_argument("--generated-root", type=Path, default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")))
    args = parser.parse_args()
    if not negative_control():
        parser.error("negative control did not detect all forbidden boundary forms")
    findings = check(args.crate)
    if not args.source_only:
        findings.extend(generated_findings(args.generated_root))
    for finding in findings:
        print(finding)
    print(f"API boundary: {len(findings)} finding(s); negative control passed")
    return bool(findings)


if __name__ == "__main__":
    raise SystemExit(main())
