#!/usr/bin/env python3
"""Split crates/horizon-mod-vocab/src/exports/part_table.rs into one export fragment per parts source file.

part_table.rs re-exported items of `crate::parts` in two wrapped brace lists that every PR adding a Parts export
edited (AGENTS.md, "No conflict hotspots"). `crate::parts` is assembled by the generator (parts_modules.rs) from a
glob re-export of each `parts/<file>.rs`, so every name has one source file. This script reads each parts file's
top-level items with a small Rust tokenizer (strings, raw strings, chars, lifetimes and comments are skipped as
tokens; nested items, `impl` blocks and bodies are skipped by bracket depth) and writes each name into
`exports/parts_<file>.rs`, in the scope, visibility and `#[cfg]` it had, with the rename and the test-only lines
kept as they were. A name a parts file brings in with `pub use super::<other>::...` is attributed to the file that
defines it (`<other>.rs`), since that is the file a PR adding such a name changes; a `pub use` of a child module of
`parts/<file>/` goes to `parts_<file>_<child>.rs`. Names the generator declares go to the fragment of where they are
declared: the union traits of `parts/unions.rs.in` to `parts_unions.rs`, and the generated `tests` and
`installation` modules of `parts/mod.rs` to `parts.rs`, which also holds `mod parts;`. A name the script cannot
attribute goes to `parts_shared.rs` and is printed. part_table.rs is deleted.

    python3 tools/refactors/split_part_table.py            # rewrite exports/
    python3 tools/refactors/split_part_table.py --dry-run  # print the plan only

Proof (read-only), run once on each tree:

    python3 tools/refactors/split_part_table.py --exports-digest TREE [--target-dir DIR]

builds `cargo check -p horizon-mod-vocab` in TREE, reads the generated `mod_api_exports_*.rs` from the crate's
OUT_DIR (taken from cargo's build-script message, not guessed), turns each into its sorted list of exported paths
(one line per leaf of every `use`, with its attributes and visibility; module path attributes made relative to TREE)
and prints a SHA-256 per file and one over all of them. The other files of that OUT_DIR (the wire code with its
layout hashes, the host tables, `parts_modules.rs`) are hashed byte for byte, with TREE's path removed, and printed
too: they must not change at all.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
VOCAB = "crates/horizon-mod-vocab/src"
PARTS = f"{VOCAB}/parts"
EXPORTS = f"{VOCAB}/exports"
OLD = f"{EXPORTS}/part_table.rs"
# Items of `crate::parts` written by the generator (crates/horizon-mod-api-gen/src/parts.rs) into parts/mod.rs's
# include, not by a parts file: their owner is `mod` (parts/mod.rs), whose fragment is parts.rs.
GENERATED = {"tests": "mod", "installation": "mod"}
UNATTRIBUTED = "shared"

# ---- Rust tokens ----------------------------------------------------------------------------------------------------

TOKEN = re.compile(
    r'''(?P<comment>//[^\n]*|/\*(?:[^*]|\*(?!/))*\*/)'''
    r'''|(?P<string>b?r(?P<hashes>#*)".*?"(?P=hashes)|b?"(?:\\.|[^"\\])*"|b?'(?:\\[^']{1,10}|[^\\'\n])')'''
    r'''|(?P<lifetime>'\w+)|(?P<word>\w+)|(?P<path>::)|(?P<punct>\S)''',
    re.S,
)
OPEN, CLOSE = "([{", ")]}"


def tokens(source):
    """Rust source as tokens, without comments and whitespace; literals stay whole."""
    return [m.group(0) for m in TOKEN.finditer(source) if not m.group("comment")]


def group_end(toks, i):
    """Index after the bracket group that opens at toks[i]."""
    depth = 0
    for j in range(i, len(toks)):
        if toks[j] in OPEN:
            depth += 1
        elif toks[j] in CLOSE:
            depth -= 1
            if depth == 0:
                return j + 1
    raise ValueError("unbalanced brackets")


def use_leaves(toks):
    """The leaves of a use tree: (path segments, bound name, or '*' for a glob), in source order."""
    out = []

    def tree(i, prefix):
        path = list(prefix)
        while True:
            t = toks[i]
            if t == "{":
                end = group_end(toks, i)
                j = i + 1
                while j < end - 1:
                    j = tree(j, path)
                    if toks[j] == ",":
                        j += 1
                return end
            if t == "*":
                out.append((path, "*"))
                return i + 1
            if t == "::":
                i += 1
                continue
            path.append(t)
            i += 1
            if toks[i] == "::":
                continue
            if toks[i] == "as":
                out.append((path, toks[i + 1]))
                return i + 2
            out.append((path, path[-1]))
            return i

    tree(0, [])
    return out


def macro_path(toks, i):
    """The tokens from toks[i] up to the first `!` or bracket, joined: `a :: b !` for a macro call."""
    j = i
    while j < len(toks) and toks[j] not in OPEN and toks[j] != "!":
        j += 1
    return " ".join(toks[i:j + 1])


def top_items(source):
    """Top-level items of a Rust file: dicts with attrs (token text), vis (None, `pub` or `pub(...)`), kind, name
    (None for `impl` and macro calls) and, for `use`, its leaves."""
    toks = tokens(source)
    items, i = [], 0
    while i < len(toks):
        attrs = []
        while toks[i] == "#":
            j = i + 1 + (toks[i + 1] == "!")
            end = group_end(toks, j)
            attrs.append(" ".join(toks[i:end]))
            i = end
        vis = None
        if toks[i] == "pub":
            vis, i = "pub", i + 1
            if toks[i] == "(":
                end = group_end(toks, i)
                vis, i = "pub(" + " ".join(toks[i + 1:end - 1]) + ")", end
        while toks[i] in ("unsafe", "async", "default", "extern") or (
                toks[i] == "const" and toks[i + 1] in ("fn", "unsafe", "async", "extern")):
            i += 2 if toks[i] == "extern" and toks[i + 1].startswith('"') else 1
        kind, start = toks[i], i
        item = {"attrs": attrs, "vis": vis, "kind": kind, "name": None}
        if kind == "use":
            end = toks.index(";", i)
            item["leaves"] = use_leaves(toks[i + 1:end] + [";"])
            i = end + 1
        elif kind in ("const", "static", "type"):
            item["name"] = toks[i + 2] if toks[i + 1] == "mut" else toks[i + 1]
            depth = 0
            while not (toks[i] == ";" and depth == 0):
                depth += toks[i] in OPEN
                depth -= toks[i] in CLOSE
                i += 1
            i += 1
        elif kind in ("fn", "struct", "enum", "union", "trait", "mod", "impl"):
            if kind != "impl":
                item["name"] = toks[i + 1]
            depth = 0
            while True:
                t = toks[i]
                if t == ";" and depth == 0:
                    i += 1
                    break
                if t in OPEN:
                    if t == "{" and depth == 0:
                        i = group_end(toks, i)
                        break
                    depth += 1
                elif t in CLOSE:
                    depth -= 1
                i += 1
        elif kind == "macro_rules" or re.fullmatch(r"\w+(?: :: \w+)* !", macro_path(toks, i)):
            while toks[i] not in OPEN:
                i += 1
            close = toks[i]
            i = group_end(toks, i)
            if close != "{" and i < len(toks) and toks[i] == ";":
                i += 1
        else:
            raise ValueError(f"unexpected token {kind!r} at top level")
        item["tokens"] = toks[start:i]
        items.append(item)
    return items


# ---- attribution --------------------------------------------------------------------------------------------------

def owners(root):
    """{name: owner} for every visible name of `crate::parts`; an owner is a parts file's stem (`blocks`), a child
    file (`shared/registration_state`), `unions` (unions.rs.in) or `mod` (parts/mod.rs and its generated include).
    A name with two owners maps to None."""
    parts = Path(root) / PARTS
    found = {}

    def add(name, owner):
        found[name] = owner if found.get(name, owner) == owner else None

    sources = [(p.stem, p) for p in sorted(parts.glob("*.rs")) if p.stem != "mod"]
    sources.append(("unions", parts / "unions.rs.in"))
    for stem, path in sources:
        for item in top_items(path.read_text(encoding="utf-8")):
            if item["vis"] is None:
                continue
            if item["kind"] != "use":
                if item["name"]:
                    add(item["name"], stem)
                continue
            for path_, name in item["leaves"]:
                if name == "*":
                    continue
                head = path_[1] if path_[0] == "super" and len(path_) > 1 else None
                child = path_[1] if path_[0] == "self" else path_[0]
                if head and (parts / f"{head}.rs").is_file():
                    add(name, head)
                elif (parts / stem / f"{child}.rs").is_file():
                    add(name, f"{stem}/{child}")
                else:
                    add(name, stem)
    for name, owner in GENERATED.items():
        add(name, owner)
    return found


def fragment_name(owner):
    return "parts.rs" if owner == "mod" else "parts_" + owner.replace("/", "_") + ".rs"


# ---- the old fragment ---------------------------------------------------------------------------------------------

def old_entries(source):
    """The `use` leaves of part_table.rs: (scope, attrs, vis, prefix, name, rename or None), in source order, and
    whether it declares `mod parts;`."""
    entries, declares = [], False

    def walk(items, scope):
        nonlocal declares
        for item in items:
            if item["kind"] == "mod":
                body = item["tokens"]
                if body[-1] == ";":
                    if body != ["mod", "parts", ";"] or item["attrs"] or item["vis"]:
                        raise ValueError(f"unexpected module declaration: {' '.join(body)}")
                    declares = True
                    continue
                if item["attrs"]:
                    raise ValueError("an export scope carries attributes")
                inner = body[body.index("{") + 1:-1]
                walk(top_items(" ".join(inner)), scope + (item["name"],))
                continue
            if item["kind"] != "use":
                raise ValueError(f"unexpected item in {OLD}: {' '.join(item['tokens'])}")
            for path, bound in item["leaves"]:
                prefix, name = "::".join(path[:-1]), path[-1]
                entries.append((scope, tuple(item["attrs"]), item["vis"], prefix, name,
                                bound if bound != name else None))

    walk(top_items(source), ())
    return entries, declares


def attr_text(attr):
    """An attribute's tokens as Rust source, before rustfmt: tokens joined without the spaces rustfmt removes."""
    text = " ".join(attr.split())
    text = re.sub(r"\s*([#\[\]()!])\s*", r"\1", text)
    return re.sub(r"\s*,\s*", ", ", text)


def render(owner, entries, declares):
    """The fragment text for one owner (rustfmt formats it afterwards)."""
    if owner == "mod":
        lines = ["//! The `parts` module, and exports of the items the generator declares in it (`parts_modules.rs`)."]
    else:
        source = f"parts/{owner}" + (".rs.in" if owner == "unions" else ".rs")
        lines = [f"//! Exports of `crate::parts` items defined in `{source}`."]
    if declares:
        lines.append("mod parts;")

    def scope_lines(scope):
        out, plain = [], {}
        for e_scope, attrs, vis, prefix, name, rename in entries:
            if e_scope != scope:
                continue
            if attrs or rename:
                out.append(("single", [attr_text(a) for a in attrs],
                            f"{vis} use {prefix}::{name}" + (f" as {rename}" if rename else "") + ";"))
            else:
                key = (vis, prefix)
                if key not in plain:
                    plain[key] = []
                    out.append(("list", key, plain[key]))
                plain[key].append(name)
        text = []
        for entry in out:
            if entry[0] == "single":
                text += entry[1] + [entry[2]]
            else:
                (vis, prefix), names = entry[1], entry[2]
                inner = names[0] if len(names) == 1 else "{" + ", ".join(names) + "}"
                text.append(f"{vis} use {prefix}::{inner};")
        return text

    lines += scope_lines(())
    scopes = sorted({e[0] for e in entries if e[0]})
    if scopes:
        if any(len(s) != 1 or s[0] != "__private" for s in scopes):
            raise ValueError(f"scopes beyond __private: {scopes}")
        lines.append("mod __private {")
        lines += ["    " + x for x in scope_lines(("__private",))]
        lines.append("}")
    return "\n".join(lines) + "\n"


def plan(root):
    """{fragment file name: text}, the names nobody defines, and fragment names changed to avoid a clash."""
    root = Path(root)
    entries, declares = old_entries((root / OLD).read_text(encoding="utf-8"))
    found = owners(root)
    by_owner, unattributed = {}, []
    for entry in entries:
        owner = found.get(entry[4])
        if owner is None:
            unattributed.append(entry[4])
            owner = UNATTRIBUTED
        by_owner.setdefault(owner, []).append(entry)
    if declares:
        by_owner.setdefault("mod", [])
    existing = {p.name for p in (root / EXPORTS).glob("*.rs")} - {"part_table.rs"}
    out, renamed = {}, []
    for owner, owned in sorted(by_owner.items()):
        name = fragment_name(owner)
        if name in existing:
            new = name[:-3] + "_table.rs"
            renamed.append((name, new))
            name = new
        out[name] = render(owner, owned, declares and owner == "mod")
    return out, sorted(set(unattributed)), renamed


def split(root, dry_run):
    fragments, unattributed, renamed = plan(root)
    for name, text in fragments.items():
        print(f"== {EXPORTS}/{name}\n{text}")
        if not dry_run:
            (Path(root) / EXPORTS / name).write_text(text, encoding="utf-8")
    for old, new in renamed:
        print(f"note: {EXPORTS}/{old} already exists; wrote {new}")
    for name in unattributed:
        print(f"unattributed (in {fragment_name(UNATTRIBUTED)}): {name}")
    if not dry_run:
        (Path(root) / OLD).unlink()
        paths = [str(Path(root) / EXPORTS / name) for name in fragments]
        subprocess.run(["rustfmt", "--edition", "2024", *paths], check=True)
    return 0


# ---- proof --------------------------------------------------------------------------------------------------------

def out_dir(tree, target_dir):
    """Builds `cargo check -p horizon-mod-vocab` in `tree`; the OUT_DIR cargo reports for its build script."""
    command = ["cargo", "check", "-q", "-p", "horizon-mod-vocab", "--message-format=json"]
    if target_dir:
        command += ["--target-dir", str(target_dir)]
    p = subprocess.run(command, cwd=tree, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(f"cargo check failed in {tree}:\n{p.stderr[-4000:]}")
    for line in p.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "build-script-executed" and "horizon-mod-vocab" in message["package_id"]:
            return Path(message["out_dir"])
    raise SystemExit(f"cargo reported no build script output for horizon-mod-vocab in {tree}")


def exported_paths(code, tree):
    """A generated export file as its sorted exported paths: one line per `use` leaf (with attributes and
    visibility), and module declarations with their path attribute relative to the tree."""
    lines = []
    for item in top_items(code.replace(str(tree), "<tree>")):
        head = " ".join(item["attrs"] + ([item["vis"]] if item["vis"] else []))
        if item["kind"] == "use":
            for path, bound in item["leaves"]:
                leaf = "::".join(path) + ("" if bound in (path[-1], "*") else f" as {bound}")
                lines.append(f"{head} use {'::'.join(path) + '::*' if bound == '*' else leaf}".strip())
        else:
            lines.append(" ".join([head] + item["tokens"]).strip())
    return sorted(lines)


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def exports_digest(tree, target_dir):
    tree = Path(tree).resolve()
    out = out_dir(tree, target_dir)
    total = hashlib.sha256()
    for path in sorted(out.glob("mod_api_exports_*.rs")):
        normal = "\n".join(exported_paths(path.read_text(encoding="utf-8"), tree)) + "\n"
        total.update(f"{path.name}\n{normal}".encode())
        print(f"{digest(normal)}  {path.name} ({normal.count(chr(10))} paths, sorted)")
    print(f"{total.hexdigest()}  all mod_api_exports_*.rs (sorted paths)")
    rest = hashlib.sha256()
    for path in sorted(p for p in out.iterdir() if p.is_file() and not p.name.startswith("mod_api_exports_")):
        text = path.read_text(encoding="utf-8").replace(str(tree), "<tree>")
        rest.update(f"{path.name}\n{text}".encode())
        print(f"{digest(text)}  {path.name} (bytes)")
    print(f"{rest.hexdigest()}  every other generated file (bytes)")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--exports-digest", metavar="TREE", help="read-only: hash TREE's generated export files")
    ap.add_argument("--target-dir", help="with --exports-digest: cargo's target directory")
    args = ap.parse_args(argv)
    if args.exports_digest:
        return exports_digest(args.exports_digest, args.target_dir)
    return split(args.root, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
