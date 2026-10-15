#!/usr/bin/env python3
"""Map vanilla classes to the Horizon code that ports them and the harnesses that prove it
(docs/version-ports.md).

Three sources, all in the repository:

- Rust comments: a CamelCase name in a `//` comment that is exactly one vanilla class's simple name (or
  `Outer.Inner`), or any class name followed by `.member`, `::member` or `#member`, maps that file (and the
  member, when named) to the class.
- Java oracles (mods/horizon/src/test/java/dev/horizonlab/parity): the vanilla classes each one imports or
  names in full; a harness uses the oracles its script names, and those its recorded parity dependencies list.
- Harness probes: the crate examples and binaries a harness script names, and the modules they `use`; the
  harness proves those files, so it covers the classes they name.

    vanilla_map.py CLASS_INDEX [CLASS...]        print what each class maps to (all classes: --all)
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
ORACLES = "mods/horizon/src/test/java/dev/horizonlab/parity"
COMMENT_NAME = re.compile(r"\b([A-Z][A-Za-z0-9]*)(?:(?:\.|::|#)([A-Za-z][A-Za-z0-9]*))?")
JAVA_IMPORT = re.compile(r"\bimport\s+(static\s+)?((?:net\.minecraft|com\.mojang)\.[\w.]+?)(\.\*)?\s*;")
JAVA_QUALIFIED = re.compile(r"\b((?:net\.minecraft|com\.mojang)(?:\.[a-z][a-z0-9_]*)+(?:\.[A-Z]\w*)+)")
RUST_USE = re.compile(r"\b(horizon_[a-z_]+|crate)((?:::[a-z_][a-z0-9_]*)+)")


def humps(name):
    return sum(c.isupper() for c in name)


class Names:
    """Vanilla class names (internal form) by simple name, with a tie-break for the ambiguous ones."""

    def __init__(self, classes):
        self.classes = set(classes)
        self.by_simple = defaultdict(set)
        self.by_package = defaultdict(set)
        for name in classes:
            package, _, local = name.rpartition("/")
            parts = local.split("$")
            if any(p[:1].isdigit() for p in parts):
                continue  # Anonymous classes.
            self.by_simple[parts[-1]].add(name)
            if len(parts) > 1:
                self.by_simple[".".join(parts[-2:])].add(name)
            else:
                self.by_package[package].add(name)

    def resolve(self, simple):
        """The one class a simple name means: unique, else the unique top-level one, else the unique
        top-level one under net/minecraft."""
        candidates = self.by_simple.get(simple, set())
        for narrowed in (candidates, {c for c in candidates if "$" not in c},
                         {c for c in candidates if "$" not in c and c.startswith("net/minecraft/")}):
            if len(narrowed) == 1:
                return next(iter(narrowed))
        return None

    def java(self, dotted):
        """net.minecraft.a.B.C -> net/minecraft/a/B$C, if that class exists."""
        parts = dotted.split(".")
        for split in range(len(parts) - 1, 0, -1):
            if not parts[split][:1].isupper() or parts[split - 1][:1].isupper():
                continue
            name = "/".join(parts[:split]) + "/" + "$".join(parts[split:])
            if name in self.classes:
                return name
        return None


def comment_text(line):
    at = line.find("//")
    return line[at:] if at >= 0 else ""


def rust_references(files, names, read=lambda f: (ROOT / f).read_text(errors="replace")):
    """{class: {file: [members named]}} from the comments of the given Rust files."""
    found = defaultdict(lambda: defaultdict(set))
    for path in files:
        for line in read(path).splitlines():
            for match in COMMENT_NAME.finditer(comment_text(line)):
                name, member = match[1], match[2]
                if member and member[0].isupper() and names.resolve(f"{name}.{member}"):
                    name, member = f"{name}.{member}", None
                if humps(name.split(".")[-1]) < 2 and not member:
                    continue
                cls = names.resolve(name)
                if cls:
                    found[cls][path].update([member] if member else [])
    return {cls: {path: sorted(members) for path, members in files.items()} for cls, files in found.items()}


def java_references(text, names):
    """The vanilla classes a Java source imports or names in full."""
    found = set()
    words = set(re.findall(r"\b[A-Z]\w*", text))
    for static, dotted, wildcard in JAVA_IMPORT.findall(text):
        if wildcard and not static:
            package = dotted.replace(".", "/")
            found.update(c for c in names.by_package.get(package, ()) if c.rsplit("/", 1)[1] in words)
        else:
            cls = names.java(dotted if not static or wildcard else dotted.rsplit(".", 1)[0])
            if cls:
                found.add(cls)
    for dotted in JAVA_QUALIFIED.findall(text):
        cls = names.java(dotted)
        if cls:
            found.add(cls)
    return found


def harness_name(script):
    return Path(script).stem.removeprefix("test_").removesuffix("_parity")


def probe_files(script_text, examples):
    """The crate examples and binaries a harness script names: {name: path}."""
    words = set(re.findall(r"[A-Za-z0-9_]+", script_text))
    return {name: path for name, path in examples.items() if name in words}


def module_files(path, text, sources):
    """The workspace source files the `use` paths of a Rust file reach (one level)."""
    crate_dir = path.split("/src/")[0] if "/src/" in path else path.split("/examples/")[0]
    reached = set()
    for crate, modules in RUST_USE.findall(text):
        base = crate_dir if crate == "crate" else "crates/" + crate.replace("_", "-")
        parts = [p for p in modules.split("::") if p]
        for depth in range(len(parts), 0, -1):
            stem = f"{base}/src/{'/'.join(parts[:depth])}"
            hit = next((c for c in (f"{stem}.rs", f"{stem}/mod.rs") if c in sources), None)
            if hit:
                reached.add(hit)
                break
    return reached


def build(classes, root=ROOT, dependencies=None):
    """{"classes": {class: {"rust": {file: [members]}, "harnesses": [...]}}, "harnesses": {harness: {...}}}.

    dependencies: {harness: [repository paths]} recorded by tools/parity.py, when available."""
    names = Names(classes)
    tracked = subprocess.check_output(["git", "ls-files"], cwd=root, text=True).split()
    rust = [f for f in tracked if f.endswith(".rs") and (f.startswith("crates/") or f.startswith("launcher/"))]
    sources = set(rust)
    read = lambda f: (root / f).read_text(errors="replace")  # noqa: E731
    by_class = rust_references(rust, names, read)
    by_file = defaultdict(set)
    for cls, files in by_class.items():
        for path in files:
            by_file[path].add(cls)
    oracles = {Path(f).stem: java_references(read(f), names)
               for f in tracked if f.startswith(ORACLES + "/") and f.endswith(".java")}
    examples = {Path(f).stem: f for f in rust if "/examples/" in f or "/src/bin/" in f}
    harnesses = {}
    for script in sorted(f for f in tracked if re.fullmatch(r"tools/test_\w+_parity\.py", f)):
        text = read(script)
        name = harness_name(script)
        used = {o for o in oracles if re.search(rf"\b{o}\b", text)}
        used |= {Path(p).stem for p in (dependencies or {}).get(name, []) if Path(p).stem in oracles}
        probes = probe_files(text, examples)
        files = set(probes.values())
        for probe in probes.values():
            files |= module_files(probe, read(probe), sources)
        covered = set().union(*(oracles[o] for o in used), *(by_file.get(f, ()) for f in files))
        harnesses[name] = {"oracles": sorted(used), "rust": sorted(files), "classes": sorted(covered)}
    mapped = defaultdict(lambda: {"rust": {}, "harnesses": []})
    for cls, files in by_class.items():
        mapped[cls]["rust"] = files
    for name, harness in harnesses.items():
        for cls in harness["classes"]:
            mapped[cls]["harnesses"].append(name)
    return {"classes": dict(sorted(mapped.items())), "harnesses": harnesses}


def lookup(mapping, cls):
    """What a class maps to, falling back to its outermost class for nested ones."""
    entry = mapping["classes"].get(cls)
    if entry is None and "$" in cls:
        entry = mapping["classes"].get(cls.split("$")[0])
    return entry or {"rust": {}, "harnesses": []}


def recorded_dependencies(shared):
    """tools/parity.py's dependency records ({harness: [paths]}), from parity-shared/dependencies."""
    directory = Path(shared) / "dependencies"
    found = {}
    for path in sorted(directory.glob("*.json")) if directory.is_dir() else []:
        try:
            found[path.stem.split("#")[0]] = json.loads(path.read_text())["dependencies"]
        except (ValueError, KeyError):
            continue
    return found


def default_shared():
    return (ROOT / ".local" / "minecraft").resolve().parent / "parity-shared"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("index", help="a class index (tools/class_index.py, versions/<id>/classes.json)")
    parser.add_argument("classes", nargs="*", help="internal or dotted names")
    parser.add_argument("--all", action="store_true", help="print the whole map")
    args = parser.parse_args()
    classes = json.loads(Path(args.index).read_text())
    mapping = build(classes, dependencies=recorded_dependencies(default_shared()))
    if args.all:
        print(json.dumps(mapping, indent=1))
        return 0
    for cls in args.classes:
        cls = cls.replace(".", "/") if "/" not in cls else cls
        print(json.dumps({cls: lookup(mapping, cls)}, indent=2))
    covered = sum(1 for entry in mapping["classes"].values() if entry["harnesses"])
    print(f"{len(mapping['classes'])} vanilla classes mapped, {covered} covered by a harness", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
