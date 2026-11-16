#!/usr/bin/env python3
"""mdBook preprocessor: the mod SDK reference's tables, rendered from what the code publishes, never kept by hand.

- `{{#sdk-assets}}`: the files of an `sdk-<build id>` release, from the list the publish job refuses to release
  without (`tools/ci/nightly.py`, `SDK_ASSETS`).
- `{{#sdk-routes}}`: which compiled slices each launcher route fetches, from the SDK's input list
  (`crates/horizon-mod-package/src/sdk/inputs.json`).
- `{{#sdk-hosts}}`: the computers an SDK is built for, from the packing legs (`nightly.py`, `SDK_LEGS`).
- `{{#sdk-surface}}`: the record kinds of `surface.json`, each with its id form and how many this commit has, from
  this checkout's own export (`horizon-stage surface`).

The words beside each kind, target, route and record prefix are here; a name that has none fails the build, and so
do words for one that is gone, so the tables never drift from what is published. A marker no chapter holds fails the
build too, so a table cannot drop out of the book unnoticed.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
INPUTS = ROOT / "crates/horizon-mod-package/src/sdk/inputs.json"

# What each asset kind holds (`horizon_mod_package::sdk::Kind`), and the files beside them.
KINDS = {
    "surface": "The stable surface: everything an API-only mod can name, each item with its fingerprint "
               "([`surface.json`](02-surface.md)).",
    "build-data": "`points.json`, `internals.json`, `loops.json` and `functions.json`: data pinned to this build, "
                  "with no compatibility promise.",
    "api-source": "The mod API's crates with their generated code written out: what a mod project's "
                  "`.horizon/sdk/api/` holds.",
    "runtime": "The game's libraries, the prebuilt program the API route links mods into, its stub mods library "
               "and the package's identity file.",
    "api": "Metadata of the mod API's crates and their procedural macros, the point data and the mods library's "
           "template: what a mod on the mod API alone builds against.",
    "engine": "Every other file of the prebuilt game: what a mod with a `[native.*]` table, game internals or a "
              "registry dependency builds against as well.",
    "index": "The index: the build, every other asset with its SHA-256, size, kind and codec "
             "([fields](01-release.md#sdkjson)).",
    "sums": "The SHA-256 of every file of the release.",
    "signature": "Horizon's signature over `SHA256SUMS` for the tag `sdk-<build id>`.",
}
BESIDE = {"sdk.json": "index", "SHA256SUMS": "sums", "SHA256SUMS.sig": "signature"}
PER_TARGET = ("runtime", "api", "engine")

TARGETS = {
    "x86_64-unknown-linux-gnu": "Linux, x86-64",
    "x86_64-pc-windows-gnu": "Windows, x86-64",
    "aarch64-apple-darwin": "macOS, Apple silicon",
}

ROUTES = {
    "api": "Every enabled mod uses the mod API alone.",
    "engine": "A mod has a `[native.*]` table, names game internals, or depends on a registry (crates.io) crate.",
}

# `surface.json`'s record kinds by id prefix (`horizon_mod_surface`, "Ids"); a member's id has a `.` after its owner.
RECORDS = {
    "hook": "Hook: a moment the game tells mods about",
    "hook member": "An argument or the output of a hook",
    "trait": "Trait of the API",
    "method": "Method of a trait",
    "assoc": "Associated type or const of a trait",
    "handle": "Inherent method or const of an API type, or of a trait object",
    "fn": "Free function",
    "macro": "Macro",
    "type": "Type",
    "type member": "Public field or variant of a type",
    "part": "Part kind",
    "capability": "Capability",
    "behaviour": "Behaviour: a promise that is no type or signature",
}
ID_FORMS = {
    "hook": "hook:<name>",
    "hook member": "hook:<name>.<argument>",
    "trait": "trait:<path>",
    "method": "method:<trait path>::<name>",
    "assoc": "assoc:<trait path>::<name>",
    "handle": "handle:<type or trait path>::<name>",
    "fn": "fn:<path>",
    "macro": "macro:<path>",
    "type": "type:<path>",
    "type member": "type:<path>.<field or variant>",
    "part": "part:<kind>",
    "capability": "capability:<key>",
    "behaviour": "behaviour:<name>",
}


def kind_of(name):
    """An asset's kind and, for a compiled slice, its target and side, as `sdk::Asset::parts_of` reads its name."""
    if name in BESIDE:
        return BESIDE[name], None, None
    if name == "surface.json":
        return "surface", None, None
    stem = name.removesuffix(".tar.xz")
    if stem in ("build-data", "api-source"):
        return stem, None, None
    for kind in PER_TARGET:
        if stem.startswith(kind + "-"):
            rest = stem[len(kind) + 1:]
            target = rest.removesuffix("-server")
            return kind, target, "server" if target != rest else "client"
    raise ValueError(f"{name}: no SDK asset kind")


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(row) + " |" for row in rows]
    return "\n".join(lines) + "\n"


def exactly(words, names, what):
    """Refuses names `words` has nothing for, and words for names that are gone."""
    names = set(names)
    missing, stale = sorted(names - set(words)), sorted(set(words) - names)
    if missing or stale:
        raise ValueError(f"{what}: no words for {missing}; words for names that are gone: {stale}")


def assets(names):
    by_kind = {}
    for name in names:
        kind, _, _ = kind_of(name)
        by_kind.setdefault(kind, []).append(name)
    exactly(KINDS, by_kind, "SDK asset kinds")
    order = list(KINDS)
    rows = []
    for kind in sorted(by_kind, key=order.index):
        files = ", ".join(f"`{name}`" for name in sorted(by_kind[kind]))
        rows.append([files, "" if kind in BESIDE.values() else f"`{kind}`", KINDS[kind]])
    return table(["File", "Kind", "Holds"], rows)


def hosts(legs):
    targets = {}
    for target, side in legs:
        targets.setdefault(target, set()).add("server" if side == "-server" else "client")
    exactly(TARGETS, targets, "SDK targets")
    rows = [[TARGETS[target], f"`{target}`", "yes" if "client" in sides else "no",
             "yes" if "server" in sides else "no"]
            for target, sides in sorted(targets.items(), key=lambda item: TARGETS[item[0]])]
    return table(["Computer", "Target", "Game", "Dedicated server"], rows)


def routes(inputs):
    with_slices = {name: route["sdk"] for name, route in inputs["routes"].items() if route.get("sdk")}
    exactly(ROUTES, with_slices, "SDK routes")
    rows = [[f"`{name}`", ROUTES[name], ", ".join(f"`{kind}`" for kind in kinds)]
            for name, kinds in sorted(with_slices.items())]
    return table(["Route", "When", "Slices it fetches"], rows)


def ids(value):
    """Every record id in a surface document, members included."""
    if isinstance(value, dict):
        if isinstance(value.get("id"), str):
            yield value["id"]
        for child in value.values():
            yield from ids(child)
    elif isinstance(value, list):
        for child in value:
            yield from ids(child)


def surface(document):
    counts = {}
    for record in ids(document):
        prefix, _, rest = record.partition(":")
        key = f"{prefix} member" if prefix in ("hook", "type") and "." in rest else prefix
        counts[key] = counts.get(key, 0) + 1
    exactly(RECORDS, counts, "surface record kinds")
    rows = [[RECORDS[key], f"`{ID_FORMS[key]}`", str(counts[key])] for key in RECORDS]
    return (f"Schema {document['schema']}. The counts are of the commit this book was built from.\n\n"
            + table(["Record", "Id", "Count"], rows))


def export_surface(root):
    """This checkout's `surface.json`, exported by its own `horizon-stage`."""
    with tempfile.TemporaryDirectory() as folder:
        out = Path(folder) / "surface.json"
        subprocess.run(["cargo", "run", "--quiet", "-p", "horizon-mod-api-gen", "--bin", "horizon-stage", "--",
                        "surface", str(root), str(out)],
                       cwd=root, check=True, stdout=sys.stderr)
        return json.loads(out.read_text())


def chapters(items):
    for item in items:
        if "Chapter" in item:
            yield item["Chapter"]
            yield from chapters(item["Chapter"]["sub_items"])


def preprocess(context, book):
    root = Path(context["root"]).resolve().parents[1]
    # As a package, the way the tests import it: run as a script, nightly.py puts tools/ first on
    # the path, which would shadow tests/ modules of the same name.
    if str(root) not in sys.path:
        sys.path.append(str(root))
    from tools.ci import nightly
    render = {
        "{{#sdk-assets}}": lambda: assets(nightly.SDK_ASSETS),
        "{{#sdk-hosts}}": lambda: hosts(nightly.SDK_LEGS),
        "{{#sdk-routes}}": lambda: routes(json.loads((root / INPUTS.relative_to(ROOT)).read_text())),
        "{{#sdk-surface}}": lambda: surface(export_surface(root)),
    }
    holding = [chapter for chapter in chapters(book["sections"])
               if any(marker in chapter["content"] for marker in render)]
    lost = sorted(marker for marker in render if not any(marker in chapter["content"] for chapter in holding))
    if lost:
        raise ValueError(f"SDK table markers no chapter holds: {lost}")
    done = {}
    for chapter in holding:
        for marker, make in render.items():
            if marker in chapter["content"]:
                if marker not in done:
                    done[marker] = make()
                chapter["content"] = chapter["content"].replace(marker, done[marker])
    return book


if __name__ == "__main__" and sys.argv[1:2] != ["supports"]:
    print(json.dumps(preprocess(*json.load(sys.stdin))))
