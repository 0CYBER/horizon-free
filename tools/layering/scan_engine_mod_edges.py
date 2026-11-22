#!/usr/bin/env python3
"""List every place an engine crate touches a mod crate, for arismoko/horizon#2534's move of the mod host above the engine.

Engine crates are the `engine` layer of tools/layers.json; mod crates are the workspace's `horizon-mod-*` crates.
The scan reads `cargo metadata` and the Rust sources, without compiling, and finds:
- manifest edges: each engine crate's dependency on a mod crate (normal, build, dev), with its Cargo.toml line;
- source edges: each path into a mod crate (`mod_api::Flow`, `horizon_mod_runtime::attachments::Holder`), as a
  `use` leaf, a re-export, an attribute, a trait impl, a call, a macro or a type, and every `include!` of a file
  the mod API generator writes;
- the move set (game/modding/, game/native_hooks/): each file's strand and phase, the inherent and foreign-trait
  impls a move out of the crate must rewrite, the engine references into it (inbound: sockets or kept items) and the
  engine-private items it uses (outbound: what the move must make public);
- the mod API's vocabulary question: which `horizon-mod-api` files are plain types and traits, and which carry a
  build-script include, `unsafe`, process state or a dependency a crate below the engine can't have.

Every edge has a strand (loading, host, slots, attachments, content_render, hooks) from the RULES table below, and
`test` when it sits in test code (tests/, examples/, a `#[cfg(test)]` or `#[test]` item, a module declared under
one), which the layer rule doesn't count. Name resolution is textual, so outbound and inbound counts are estimates:
see docs/layering/mod-host-map.md for how to read them.

    python3 tools/layering/scan_engine_mod_edges.py --json out.json      # the full scan, deterministic
    python3 tools/layering/scan_engine_mod_edges.py --map tables.md      # write the map's tables (not committed)
    python3 tools/layering/scan_engine_mod_edges.py                      # print them
"""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rust_text  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
STRANDS = ["loading", "host", "slots", "attachments", "content_render", "hooks"]
# Each strand's arismoko/horizon#2534 phase; None where nothing moves: the engine keeps its Host impls (phase 2 dropped) and attachment
# storage is already engine-layer (phase 4 dropped), owner 2026-11-10.
PHASE = {"loading": 1, "host": None, "slots": 3, "attachments": None, "content_render": 5, "hooks": 6}
# The host areas the scan inventories, as repo paths.
MOVE_SET = ("crates/horizon-client/src/game/modding/", "crates/horizon-client/src/game/native_hooks/")
GAME = "crates/horizon-client/src/game"
# horizon-client-lib's library root sits in horizon-client's src/; these paths there are horizon-client's own.
CLIENT_SRC = "crates/horizon-client/src/"
CLIENT_OWN = re.compile(r"crates/horizon-client/src/(game(\.rs|/)|entry\.rs|main\.rs|lib\.rs)")

# (strand, pattern on the edge's path "crate::a::b"); the first match wins. Owners correct strands here.
RULES = [
    ("loading", r"^(horizon_mod_package|horizon_mod_wasm|horizon_mod_deps)\b"),
    ("loading", r"^mod_api::(ModEntry|Mods|ModFields|__private::(install|mods|set_mod_enabled|ModPhase|PhaseOwner"
                r"|current_phase|LoadAccess|check_access|with_test_packages|loaded_mods|LoadedMods"
                r"|installed_code_ids|mod_index\w*|mark_game_thread|command_argument_types))\b"),
    ("slots", r"^(mod_points|horizon_mod_api_macros|horizon_mod_api_gen)\b"),
    ("attachments", r"^horizon_mod_runtime::(attachments|impl_attachment_lifetime)\b"),
    ("attachments", r"^mod_api::(Attachment\w*|attachment_queries|__private::attach\w*)\b"),
    ("host", r"^mod_api::(\w*Host|HostSide|Backend\w*|__private::(backend\w*|game))\b"),
    ("content_render", r"^mod_render\b|^horizon_mod_runtime\b"),
    ("content_render", r"^mod_api::(content|client|sprite_source|RegistrationContext|Registr\w*|Config\w*"
                       r"|__private::(\w+_key(_id|_by_\w+)?|\w+_id|effect_index|resolve_\w+|block_definitions"
                       r"|tag_\w+))\b"),
    ("hooks", r"^mod_api::"),
]
RULES = [(strand, re.compile(pattern)) for strand, pattern in RULES]
# Generated files an engine crate includes, by strand.
GENERATED = {
    "game_modules.rs": "slots", "lan_action_modules.rs": "slots", "native_hook_modules.rs": "hooks",
    "mod_api_fire_generated.rs": "hooks", "mod_api_goal_view_generated.rs": "hooks",
    "game_host_modules.rs": "host",
}
# File rules for the move set, before the edges' majority decides.
FILE_RULES = [
    # api_values is the engine↔mod_api value conversion the hook bodies use: it stays with them.
    ("hooks", re.compile(r"/game/modding/api_values")),
    ("host", re.compile(r"/game/(modding/mod_api_host|host/)")),
    ("attachments", re.compile(r"/game/modding/(attachment_\w+|entity_initialization)")),
    ("loading", re.compile(r"/game/modding/mods/")),
    ("hooks", re.compile(r"/game/native_hooks/")),
]
OUT_DIR_INCLUDE = re.compile(r'include!\(\s*concat!\(\s*env!\(\s*"OUT_DIR"\s*\)\s*,\s*"/([\w.]+)"\s*\)\s*\)')
FN_DEF = re.compile(r"(?<![\w])(pub(?:\s*\([^)]*\))?\s+)?(?:const\s+|async\s+|unsafe\s+|extern\s+)*fn\s+(\w+)")
TYPE_DEF = re.compile(r"(?<![\w])(pub(?:\s*\([^)]*\))?\s+)?(struct|enum|trait|type|union|const|static|mod)\s+(\w+)")
METHOD_CALL = re.compile(r"(?:\.|\bSelf::|\bGame::|\bGameLevel::)\s*(\w+)\s*(?:::\s*<[^;{}]*?>\s*)?\(")
FIELD = re.compile(r"\b(self|game|level|g)\s*\.\s*([a-z_]\w*)\b(?!\s*(?:\(|::\s*<))")
IMPL = re.compile(r"(?<![\w])impl\b")
STATE = re.compile(r"(?<!['\w])(static\s+(?:mut\s+)?\w+|thread_local!|OnceLock|LazyLock|OnceCell|Mutex|RwLock"
                   r"|Atomic[A-Z]\w*|RefCell|Cell\s*<)")


def git_head():
    out = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=ROOT)
    return out.stdout.strip()


def metadata():
    out = subprocess.run(["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
                         check=True, capture_output=True, text=True, cwd=ROOT).stdout
    return json.loads(out)["packages"]


def rel(path):
    return Path(path).resolve().relative_to(ROOT).as_posix()


def strand_of(path):
    for strand, pattern in RULES:
        if pattern.search(path):
            return strand
    return "hooks"


class Workspace:
    """The packages, their layers, the mod crates' names in code and the engine crates' source files."""

    def __init__(self, packages, layers):
        self.packages = {p["name"]: p for p in packages}
        self.layers = layers["crates"]
        self.mod_crates = sorted(n for n in self.packages if n.startswith("horizon-mod-"))
        self.idents = {}
        for name in self.mod_crates:
            for target in self.packages[name]["targets"]:
                if {"lib", "proc-macro", "rlib", "dylib"} & set(target["kind"]):
                    self.idents[target["name"].replace("-", "_")] = name
        self.ident_of = {name: ident for ident, name in self.idents.items()}
        # horizon-mod-package and horizon-mod-runtime sit in the engine layer but are mod crates: not scanned.
        self.engine = sorted(n for n in self.packages if self.layers.get(n) == "engine" and n not in self.mod_crates)

    def files(self):
        """{repo path: owning engine crate} for every .rs file of every engine crate's targets."""
        roots = {}
        for name in self.engine:
            package = self.packages[name]
            manifest_dir = Path(package["manifest_path"]).parent
            for target in package["targets"]:
                src = Path(target["src_path"])
                root = src if "custom-build" in target["kind"] else src.parent
                if (ROOT / rel(root)).exists():
                    owner = name if manifest_dir in src.parents or src == manifest_dir else None
                    roots.setdefault(rel(root), owner or name)
        out = {}
        for root, owner in sorted(roots.items()):
            path = ROOT / root
            for file in [path] if path.is_file() else sorted(path.rglob("*.rs")):
                repo = rel(file)
                if repo in out and out[repo] != owner and not repo.startswith(CLIENT_SRC):
                    continue
                if repo.startswith(CLIENT_SRC):
                    owner_here = "horizon-client" if CLIENT_OWN.match(repo) else "horizon-client-lib"
                else:
                    owner_here = owner
                out[repo] = owner_here
        return out


def manifest_edges(ws):
    out = []
    for name in ws.engine:
        package = ws.packages[name]
        lines = Path(package["manifest_path"]).read_text(encoding="utf-8").splitlines()
        for dep in package["dependencies"]:
            if not dep.get("path") or dep["name"] not in ws.mod_crates:
                continue
            kind = dep.get("kind") or "normal"
            section = {"normal": "[dependencies]", "build": "[build-dependencies]",
                       "dev": "[dev-dependencies]"}[kind]
            line, current = 0, None
            for i, text in enumerate(lines, 1):
                if text.startswith("["):
                    current = text.strip()
                if current and current.endswith(section) and re.match(rf"{re.escape(dep['name'])}\s*=", text):
                    line = i
                    break
            out.append({
                "crate": name, "dep": dep["name"], "kind": kind, "dep_layer": ws.layers.get(dep["name"]),
                "layer_exception": ws.layers.get(dep["name"]) != "engine" and kind != "dev",
                "file": rel(package["manifest_path"]), "line": line,
                "strand": strand_of(ws.ident_of[dep["name"]] + "::"),
            })
    return sorted(out, key=lambda e: (e["crate"], e["dep"], e["kind"]))


class Source:
    """One file: its blanked code, line table and test regions."""

    def __init__(self, path, crate):
        self.path, self.crate = path, crate
        self.text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
        self.code = rust_text.blank(self.text)
        self.starts = rust_text.line_starts(self.text)
        self.regions, self.test_mods = rust_text.test_regions(self.code)
        self.test_file = False

    def line(self, offset):
        return rust_text.line_of(self.starts, offset)

    def is_test(self, offset):
        return self.test_file or rust_text.in_regions(self.regions, offset)


def module_dir(path):
    """The directory a file's child modules live in."""
    p = Path(path)
    return p.parent if p.name in ("mod.rs", "lib.rs", "main.rs") or p.name.endswith("_lib.rs") else p.with_suffix("")


def mark_test_files(sources):
    """Files under tests/, examples/ or benches/, named *_tests.rs, declared or included under a test attribute,
    and their child modules."""
    by_path = {s.path: s for s in sources}
    for s in sources:
        parts = Path(s.path).parts
        if any(p in ("tests", "examples", "benches") for p in parts) or s.path.endswith("_tests.rs"):
            s.test_file = True
    for s in sources:
        base = module_dir(s.path)
        for name in s.test_mods:
            for child in (base / f"{name}.rs", base / name / "mod.rs"):
                if child.as_posix() in by_path:
                    by_path[child.as_posix()].test_file = True
        for inc in rust_text.INCLUDE.finditer(s.text):
            if s.is_test(inc.start()):
                child = (Path(s.path).parent / inc.group(1)).as_posix()
                if child in by_path:
                    by_path[child].test_file = True
    for s in sorted(sources, key=lambda s: len(Path(s.path).parts)):
        parent = Path(s.path).parent
        if Path(s.path).name == "mod.rs":
            parent = parent.parent / parent.name
        for candidate in (parent.with_suffix(".rs"), parent / "mod.rs"):
            owner = by_path.get(candidate.as_posix())
            if owner is not None and owner is not s and owner.test_file:
                s.test_file = True


def kind_at(code, start, end):
    """How the path at code[start:end] is used: attr, impl, macro, call or type."""
    line_start = code.rfind("\n", 0, start) + 1
    attr = code.rfind("#[", line_start, start)
    if attr >= 0 and code.find("]", attr, start) < 0:
        return "attr"
    rest = code[end:end + 200]
    if rest.lstrip().startswith("!"):
        return "macro"
    stop = max(code.rfind(c, 0, start) for c in ";{}")
    before = code[stop + 1:start]
    if IMPL.search(before) and re.match(r"\s*(<[^{;]*?>)?\s+for\b", rest):
        return "impl"
    if re.match(r"\s*(::\s*<[^;{}]*?>\s*)?\(", rest):
        return "call"
    return "type"


def path_at(code, start):
    m = re.compile(r"\w+(\s*::\s*\w+)*").match(code, start)
    return re.sub(r"\s+", "", m.group()), m.end()


def impl_self(code, end):
    m = re.compile(r"\s*(?:<[^{;]*?>)?\s+for\s+([^{]+?)\s*(?:where\b|\{)").match(code, end)
    return re.sub(r"\s+", " ", m.group(1)).strip() if m else None


def source_edges(ws, src):
    """The file's edges into mod crates, and its {local name: mod path} imports."""
    idents = ws.idents
    edges, imports, covered = [], {}, []
    code = src.code

    def add(kind, path, offset, **extra):
        edges.append({"crate": src.crate, "file": src.path, "line": src.line(offset), "kind": kind,
                      "mod_crate": idents[path.split("::")[0]], "path": path, "strand": strand_of(path),
                      "test": src.is_test(offset), **extra})

    for use in rust_text.USE.finditer(code):
        for segs, name in rust_text.use_leaves(use.group(2)):
            segs = segs[1:] if segs[0] == "" else segs
            if segs[0] not in idents:
                continue
            path = "::".join(segs)
            if name:
                imports[name] = path
            add("reexport" if use.group(1) else "use", path + ("" if name else "::*"), use.start())
        covered.append((use.start(), use.end()))
    pattern = re.compile(r"(?<![\w:])(?:::\s*)?(" + "|".join(map(re.escape, sorted(idents))) + r")\b(?=\s*::)")
    for m in pattern.finditer(code):
        if rust_text.in_regions(covered, m.start()):
            continue
        path, end = path_at(code, m.start(1))
        kind = kind_at(code, m.start(), end)
        extra = {"self_type": impl_self(code, end)} if kind == "impl" else {}
        add(kind, path, m.start(), **extra)
    for m in re.finditer(r"(?<![\w])extern\s+crate\s+(\w+)", code):
        if m.group(1) in idents:
            add("use", m.group(1), m.start())
    # Trait impls through an imported name: `use mod_api::Host; impl Host for Game<'_>`.
    for m in re.finditer(r"(?<![\w])impl\b\s*(?:<[^{;]*?>)?\s*(\w+)\b(?=\s*(?:<[^{;]*?>)?\s+for\b)", code):
        if m.group(1) in imports:
            add("impl", imports[m.group(1)], m.start(), self_type=impl_self(code, m.end()))
    for m in OUT_DIR_INCLUDE.finditer(src.text):
        if code[m.start()] == "i" and m.group(1) in GENERATED:
            edges.append({"crate": src.crate, "file": src.path, "line": src.line(m.start()), "kind": "generated",
                          "mod_crate": "horizon-mod-api-gen", "path": m.group(1), "strand": GENERATED[m.group(1)],
                          "test": src.is_test(m.start())})
    # How often each imported name is used in the file outside `use` lines.
    for edge in edges:
        if edge["kind"] in ("use", "reexport") and not edge["path"].endswith("::*"):
            name = next((n for n, p in imports.items() if p == edge["path"]), None)
            if name:
                uses = sum(1 for u in re.finditer(rf"(?<![\w]){re.escape(name)}\b", code)
                           if not rust_text.in_regions(covered, u.start()))
                edge["bare_uses"] = uses
    return edges, imports


class Definitions:
    """Names the engine sources define, with their visibility and file, for textual resolution."""

    def __init__(self, sources):
        self.fns, self.types, self.fields = defaultdict(list), defaultdict(list), defaultdict(list)
        for s in sources:
            for m in FN_DEF.finditer(s.code):
                self.fns[m.group(2)].append((s.path, (m.group(1) or "").strip()))
            for m in TYPE_DEF.finditer(s.code):
                self.types[m.group(3)].append((s.path, (m.group(1) or "").strip()))
            for m in re.finditer(r"(?<![\w])(pub(?:\s*\([^)]*\))?\s+)?struct\s+(Game|GameLevel)\b[^{;]*\{", s.code):
                end, _ = rust_text.item_end(s.code, m.start())
                body = s.code[m.end():end]
                for f in re.finditer(r"(?:^|,)\s*(pub(?:\s*\([^)]*\))?\s+)?([a-z_]\w*)\s*:(?!:)", body, re.M):
                    self.fields[f.group(2)].append((m.group(2), (f.group(1) or "").strip()))


def in_move_set(path):
    return path.startswith(MOVE_SET)


def public(defs):
    return any(vis == "pub" for _, vis in defs)


def move_set_analysis(sources, edges_by_file, defs):
    """Per move-set file: strand, phase, impls to rewrite, outbound engine-private names. Plus inbound edges."""
    files = {}
    game_sources = [s for s in sources if s.crate == "horizon-client" and s.path.startswith(GAME)]
    moved = [s for s in game_sources if in_move_set(s.path)]
    for s in moved:
        strand = next((st for st, rule in FILE_RULES if rule.search(s.path)), None)
        counts = Counter(e["strand"] for e in edges_by_file.get(s.path, []) if not e["test"])
        if strand is None:
            strand = max(STRANDS, key=lambda st: (counts[st], -STRANDS.index(st))) if counts else "hooks"
        inherent = len(re.findall(r"(?<![\w])impl\b\s*(?:<[^{;]*?>)?\s*(?:super::|crate::game::)?"
                                  r"(?:Game|GameLevel)\s*<[^{;]*?>\s*(?:where\b[^{]*)?\{", s.code))
        foreign = sum(1 for e in edges_by_file.get(s.path, []) if e["kind"] == "impl" and not e["test"])
        outbound = set()
        for m in METHOD_CALL.finditer(s.code):
            name = m.group(1)
            if s.is_test(m.start()) or any(in_move_set(p) for p, _ in defs.fns.get(name, [])):
                continue
            outside = [d for d in defs.fns.get(name, []) if not in_move_set(d[0])]
            if outside and not public(outside) and all(p.startswith(GAME) for p, _ in outside):
                outbound.add(("fn", name))
        for m in FIELD.finditer(s.code):
            name = m.group(2)
            if not s.is_test(m.start()) and name in defs.fields and not public(defs.fields[name]):
                outbound.add(("field", name))
        for m in re.finditer(r"(?<![\w])(?:super|crate::game|game)\s*::\s*(?:\{([^;]*?)\}|(\w+))", s.code):
            names = re.findall(r"\b([A-Za-z_]\w*)\b", m.group(1) or m.group(2))
            for name in names:
                if s.is_test(m.start()) or name in ("self", "super", "crate", "as"):
                    continue
                defs_here = defs.types.get(name, []) + defs.fns.get(name, [])
                outside = [d for d in defs_here if not in_move_set(d[0])]
                if outside and not public(outside) and all(p.startswith(GAME) for p, _ in outside):
                    outbound.add(("item", name))
        files[s.path] = {
            "strand": strand, "phase": PHASE[strand], "lines": s.text.count("\n"), "test_file": s.test_file,
            "inherent_impls": inherent, "foreign_trait_impls": foreign,
            "outbound": sorted(f"{k}:{n}" for k, n in outbound),
        }
    # Inbound: engine code outside the move set that names a module, type or function only the move set defines.
    move_modules = set()
    for s in moved:
        p = Path(s.path)
        name = p.parent.name if p.name == "mod.rs" else p.stem
        if p.parent.name in ("modding", "native_hooks") or p.name == "mod.rs":
            move_modules.add(name)
    move_modules |= {"native_hooks", "modding"}
    elsewhere = {Path(s.path).stem for s in sources if not in_move_set(s.path)}
    elsewhere |= {Path(s.path).parent.name for s in sources if not in_move_set(s.path) and s.path.endswith("mod.rs")}
    move_modules -= elsewhere - {"native_hooks", "modding"}
    only_fns = {n for n, d in defs.fns.items() if all(in_move_set(p) for p, _ in d)}
    only_types = {n for n, d in defs.types.items() if all(in_move_set(p) for p, _ in d) and n[:1].isupper()}
    owner_fn = {n: sorted(p for p, _ in defs.fns[n])[0] for n in only_fns}
    owner_ty = {n: sorted(p for p, _ in defs.types[n])[0] for n in only_types}
    inbound = []
    for s in game_sources:
        if in_move_set(s.path):
            continue
        for m in METHOD_CALL.finditer(s.code):
            if m.group(1) in only_fns:
                inbound.append((s, m.start(), "call", m.group(1), owner_fn[m.group(1)]))
        for m in re.finditer(r"(?<![\w])(\w+)\s*::", s.code):
            if m.group(1) in move_modules:
                inbound.append((s, m.start(), "module", m.group(1), None))
        for m in re.finditer(r"(?<![\w.])([A-Z]\w*)\b", s.code):
            if m.group(1) in only_types:
                inbound.append((s, m.start(), "type", m.group(1), owner_ty[m.group(1)]))
    module_owner = {}
    for path in files:
        p = Path(path)
        module_owner.setdefault(p.parent.name if p.name == "mod.rs" else p.stem, path)
    out = []
    for s, offset, kind, name, owner in inbound:
        owner = owner or module_owner.get(name)
        strand = files[owner]["strand"] if owner in files else "hooks"
        out.append({"crate": s.crate, "file": s.path, "line": s.line(offset), "kind": kind, "name": name,
                    "defined_in": owner, "strand": strand, "test": s.is_test(offset)})
    out.sort(key=lambda e: (e["file"], e["line"], e["kind"], e["name"]))
    return files, out


def vocabulary(ws):
    """Per `horizon-mod-api` source file: what keeps it from a crate below the engine."""
    package = ws.packages["horizon-mod-api"]
    base = Path(package["manifest_path"]).parent
    deps = sorted({(d["name"], d.get("kind") or "normal", ws.layers.get(d["name"]))
                   for d in package["dependencies"] if d.get("path")})
    crate_names = {ident: name for ident, name in ws.idents.items() if name != "horizon-mod-api"}
    crate_names["horizon_core"] = "horizon-core"
    files, sources = [], [Source(rel(path), "horizon-mod-api") for path in sorted((base / "src").rglob("*.rs"))]
    mark_test_files(sources)
    for s in sources:
        row = {"file": s.path, "lines": s.text.count("\n"), "generated_include": [], "unsafe": [], "state": [],
               "crates": defaultdict(list)}
        for m in OUT_DIR_INCLUDE.finditer(s.text):
            if s.code[m.start()] == "i":
                row["generated_include"].append(s.line(m.start()))
        for m in re.finditer(r"(?<![\w])unsafe\b", s.code):
            row["unsafe"].append(s.line(m.start()))
        for m in STATE.finditer(s.code):
            if not s.is_test(m.start()):
                row["state"].append(s.line(m.start()))
        for m in re.finditer(r"(?<![\w:])(" + "|".join(map(re.escape, sorted(crate_names))) + r")\s*::", s.code):
            row["crates"][crate_names[m.group(1)]].append(s.line(m.start()))
        row["state"] = sorted(set(row["state"]))
        row["crates"] = {k: sorted(set(v)) for k, v in sorted(row["crates"].items())}
        row["test"] = s.test_file
        row["pure"] = not (row["generated_include"] or row["unsafe"] or row["state"]
                           or set(row["crates"]) - {"horizon-core"})
        files.append(row)
    return {"deps": [{"name": n, "kind": k, "layer": layer} for n, k, layer in deps],
            "build_script": (base / "build.rs").is_file(), "files": files}


def scan():
    layers = json.loads((ROOT / "tools" / "layers.json").read_text(encoding="utf-8"))
    ws = Workspace(metadata(), layers)
    sources = [Source(path, crate) for path, crate in sorted(ws.files().items())]
    mark_test_files(sources)
    edges, by_file = [], defaultdict(list)
    for s in sources:
        found, _ = source_edges(ws, s)
        edges.extend(found)
        by_file[s.path].extend(found)
    files, inbound = move_set_analysis(sources, by_file, Definitions(sources))
    return {
        "head": git_head(), "engine_crates": ws.engine, "mod_crates": ws.mod_crates,
        "allowed_mod_rows": sorted(r for r in layers.get("allowed", []) if "horizon-mod" in r.split(" -> ")[0]
                                   or "horizon-mod" in r.split(" -> ")[1]),
        "manifest_edges": manifest_edges(ws), "edges": edges, "move_set": files, "inbound": inbound,
        "vocabulary": vocabulary(ws),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--json", help="write the full scan here")
    parser.add_argument("--map", help="write the tables docs/layering/mod-host-map.md describes to this file")
    args = parser.parse_args(argv)
    from mod_host_map import render  # noqa: E402  (the map's tables, kept beside the scanner)
    result = scan()
    if args.json:
        Path(args.json).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    if args.map:
        out = Path(args.map)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(f"# Mod-host map tables at {result['head']}\n\n{render(result)}\n", encoding="utf-8")
    if not (args.json or args.map):
        print(render(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
