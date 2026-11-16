#!/usr/bin/env python3
"""`verify-static`: hand-written test stand-ins keep up with the API they mirror, checked on the member PR itself.

Two test crates compile hand-written stand-ins for generated mod-API items, and a contributor's push builds neither
(it runs only `format` and `verify-static`), so a missing stand-in used to surface first in a rollup's `verify`.
This script reads source text only (no build) and fails on:

1. Dispatch-test stand-in files (`dispatch-tests/src/fixtures/stubs/<Name>.rs`, `fixtures/api/<Name>.rs`): a file
   that does not define the one item it is named after (its own impls and `use` lines may sit beside it); a name with
   a file in both folders; and a `shared.rs` that defines stand-ins in `mod stubs` (only `use` lines stay there) or
   has a `mod api` again.
2. A type named in a hook's signature (`crates/horizon-mod-vocab/src/hooks/*.rs`) that no dispatch-test fixture
   defines or re-exports.
3. The wire loopback test (its root and `tests/wire_loopback/*.rs`): an `impl Trait for X` of a vocabulary trait
   that lacks a required (bodiless) method, associated type or const of that trait; and an `impl` of a generated
   union trait (`CapabilityProviders`, `Registrar`) for X without an `impl` of every member trait for X. The union's
   members are the `pub trait *CapabilityProviders` / `*Registrar` in `crates/horizon-mod-vocab/src/parts/*.rs`,
   as `horizon-mod-api-gen/src/parts.rs` bounds them. `#[host_area]` traits are skipped: their methods get generated
   defaults (`impl_host_areas!`).

Known false negatives (these still surface only in the rollup's build):
- names the generator introduces rather than the hook signature: `Answered<T>` from `answered_by`, an `out =`
  output type such as `StandaloneModelForm`, `BlockEntityTickReply` from tick dispatch;
- a derive or a method the generated dispatch needs on an existing stub (`#[derive(Clone)]`,
  `__begin_tick_listener`), and a stand-in whose shape (fields, variants, generics) no longer fits;
- a vocabulary trait name defined more than once (each hook file declares its own `Hooks`): not checked;
- an `impl` written by a macro in the loopback;
- a required trait item with a `#[cfg(...)]` attribute: the check cannot evaluate the condition, so it never
  requires such an item.
- names are matched by their last segment: a stand-in, trait or impl target is one name across the files read, so
  two different types of the same name in different modules count as one;
- the source scanner does not nest block comments: `/* a /* b */ c */` ends at the first `*/`, and its tail is
  read as code.

Every input path above must exist and yield at least one hook and one union member, or the check fails naming the
path: a rename never turns a check off silently. To run it on a tree from before the loopback moved into its own
crate, point --loopback at the old root:

    python3 tools/hotspots/check_stand_ins.py [--root DIR] [--loopback crates/horizon-mod-wasm/tests/wire_loopback.rs]
"""

import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = "crates/horizon-mod-api-gen/dispatch-tests/src/fixtures"
FOLDERS = ("stubs", "api")
HOOKS = "crates/horizon-mod-vocab/src/hooks"
VOCAB = "crates/horizon-mod-vocab/src"
PARTS = "crates/horizon-mod-vocab/src/parts"
UNIONS = ("CapabilityProviders", "Registrar")
LOOPBACK = "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback.rs"
# Names a hook signature may use without a fixture: the prelude, std and generic placeholders the generator keeps.
STD = {"Option", "Some", "None", "Vec", "String", "Box", "Result", "Ok", "Err", "Arc", "Rc", "Self", "HashMap",
       "BTreeMap", "HashSet", "BTreeSet", "Cow", "Duration", "PhantomData", "Fn", "FnMut", "FnOnce", "Send", "Sync"}
NAMED = re.compile(r"\b(struct|enum|trait|type|union|fn|mod|const|static)\s+([A-Za-z_]\w*)")


def strip(source):
    """Source with comments and the contents of string and char literals blanked, so braces and words come only
    from code. Lengths and line breaks are kept: an index into the result is an index into `source`."""
    out, i, n = [], 0, len(source)

    def blank(text):
        return re.sub(r"[^\n]", " ", text)

    while i < n:
        c = source[i]
        word_start = i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")
        raw = re.match(r'b?r(#*)"', source[i:]) if word_start and c in "br" else None
        char = re.match(r"'(?:\\[^']{1,10}|[^\\'\n])'", source[i:]) if c == "'" else None
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end < 0 else end
        elif source.startswith("/*", i):
            end = source.find("*/", i + 2)
            end = n if end < 0 else end + 2
        elif raw:
            close = source.find('"' + raw.group(1), i + raw.end())
            end = n if close < 0 else close + 1 + len(raw.group(1))
            out.append(source[i:i + raw.end()] + blank(source[i + raw.end():end - 1 - len(raw.group(1))])
                       + source[end - 1 - len(raw.group(1)):end])
            i = end
            continue
        elif c == '"':
            end = i + 1
            while end < n and source[end] != '"':
                end += 2 if source[end] == "\\" else 1
            end = min(end + 1, n)
            out.append('"' + blank(source[i + 1:end - 1]) + source[end - 1:end])
            i = end
            continue
        elif char:
            end = i + char.end()
            out.append("'" + " " * (char.end() - 2) + "'")
            i = end
            continue
        else:
            out.append(c)
            i += 1
            continue
        out.append(blank(source[i:end]))
        i = end
    return "".join(out)


def block_end(text, open_index):
    """Index just past the brace that closes the one at `open_index`."""
    depth = 0
    for i in range(open_index, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(text)


def spans(text, start=0, end=None):
    """(start, end) of each item at brace depth 0 of `text[start:end]` (stripped text), split at `;` or a closing
    brace; leading blanks are not part of an item."""
    end = len(text) if end is None else end
    items, begin, depth = [], start, 0
    for i in range(start, end):
        c = text[i]
        if c in "{([":
            depth += 1
        elif c in "})]":
            depth -= 1
            # A block item ends at its brace; `static X: T = T { .. };` ends at the `;` after it.
            if depth == 0 and c == "}" and not re.match(r"\s*;", text[i + 1:end]):
                items.append((begin, i + 1))
                begin = i + 1
        elif c == ";" and depth == 0:
            items.append((begin, i + 1))
            begin = i + 1
    result = []
    for a, b in items:
        while a < b and text[a].isspace():
            a += 1
        if a < b:
            result.append((a, b))
    return result


def top_level(text):
    return [text[a:b] for a, b in spans(text)]


def without_attributes(item):
    return re.sub(r"^\s*(#!?\[(?:[^\[\]]|\[[^\]]*\])*\]\s*)*", "", item)


def item_name(item):
    """The name an item defines, or None for `impl`, `use` and macro calls."""
    body = without_attributes(item)
    body = re.sub(r"^pub(\([^)]*\))?\s+", "", body)
    match = re.match(r"(?:unsafe\s+)?(struct|enum|trait|type|union|fn|mod|const|static)\s+([A-Za-z_]\w*)", body)
    return match.group(2) if match else None


def defined_names(text):
    names = {m.group(2) for m in NAMED.finditer(text)}
    for use in re.findall(r"\buse\s+([^;]*);", text):
        names |= set(re.findall(r"\bas\s+([A-Za-z_]\w*)", use))
        names |= set(re.findall(r"\b([A-Za-z_]\w*)\s*(?=[,}]|$)", use.strip()))
    return names


def module_body(text, name):
    match = re.search(rf"(?<![\w:])mod\s+{name}\s*\{{", text)
    if not match:
        return None
    return text[match.end() - 1 + 1:block_end(text, match.end() - 1) - 1]


def missing_inputs(root, loopback):
    """The input paths that do not exist: each would otherwise make its check find nothing."""
    paths = [FIXTURES, *(f"{FIXTURES}/{folder}" for folder in FOLDERS), f"{FIXTURES}/shared.rs", HOOKS, PARTS,
             loopback]
    return [f"{path}: missing; tools/hotspots/check_stand_ins.py reads it. If it moved, update the script's path."
            for path in paths if not (root / path).exists()]


def check_fixture_files(root):
    problems, owners = [], {}
    fixtures = root / FIXTURES
    for folder in FOLDERS:
        for path in sorted((fixtures / folder).glob("*.rs")):
            rel = path.relative_to(root)
            owners.setdefault(path.stem, []).append(rel)
            named = [n for n in map(item_name, top_level(strip(path.read_text()))) if n]
            if named != [path.stem]:
                found = ", ".join(f"`{n}`" for n in named) or "nothing"
                problems.append(f"{rel}: a stand-in file defines exactly the item it is named after, `{path.stem}`; "
                                f"it defines {found}. Give each other item its own file in {FIXTURES}/{folder}/.")
    for name, paths in sorted(owners.items()):
        if len(paths) > 1:
            problems.append(f"`{name}` has a stand-in in both {paths[0]} and {paths[1]}: two definitions are two "
                            "different types to the generated dispatch. Keep one and re-export it with `use` "
                            "from the other scope.")
    shared = fixtures / "shared.rs"
    text = strip(shared.read_text())
    stubs = module_body(text, "stubs")
    if stubs is not None:
        for item in top_level(stubs):
            name = item_name(item)
            if name:
                problems.append(f"{shared.relative_to(root)}: `mod stubs` defines `{name}`; only `use` lines "
                                f"stay there. Move it to {FIXTURES}/stubs/{name}.rs.")
    if module_body(text, "api") is not None:
        problems.append(f"{shared.relative_to(root)}: `mod api` is back; each API stand-in has its own file, "
                        f"{FIXTURES}/api/<Name>.rs.")
    return problems


HOOK = re.compile(r"#\[hook\((?:[^()]|\([^()]*\))*\)\]\s*fn\s+(\w+)\s*(<[^>]*>)?\s*\(")


def signature(text, start):
    """The parameters and return type of the fn whose `(` is at `start - 1`: up to its body or `;`."""
    depth = 1
    for i in range(start, len(text)):
        if text[i] in ";{" and depth == 0:
            return text[start:i]
        if text[i] in "([<{":
            depth += 1
        elif text[i] in ")]>}" and not (text[i] == ">" and text[i - 1] == "-"):
            depth -= 1
    return text[start:]


def check_hook_types(root):
    fixtures = root / FIXTURES
    known = set(STD)
    for path in fixtures.rglob("*.rs"):
        known |= defined_names(strip(path.read_text()))
    hook_files = sorted((root / HOOKS).glob("*.rs"))
    texts = {path: strip(path.read_text()) for path in hook_files}
    for text in texts.values():
        known |= {m.group(2) for m in NAMED.finditer(text)}
    if not any(HOOK.search(text) for text in texts.values()):
        return [f"{HOOKS}: no `#[hook(...)] fn` found; the hook-signature check would check nothing. If the "
                "declaration form changed, update tools/hotspots/check_stand_ins.py."]
    missing = {}
    for path, text in texts.items():
        for match in HOOK.finditer(text):
            generics = set(re.findall(r"\b([A-Z]\w*)", match.group(2) or ""))
            for name in re.findall(r"\b([A-Z]\w*)", signature(text, match.end())):
                if name not in known and name not in generics:
                    missing.setdefault(name, []).append((match.group(1), path.relative_to(root)))
    return [f"{hooks[0][1]}: hook `{hooks[0][0]}` names type `{name}`, which has no dispatch-test stand-in"
            + (f" (also hooks {', '.join(f'`{h}`' for h, _ in hooks[1:])})" if len(hooks) > 1 else "")
            + f". Add {FIXTURES}/api/{name}.rs (a mod_api type; stubs/{name}.rs if the generated code names an "
            "engine stub), or a `use` re-export of an existing one."
            for name, hooks in sorted(missing.items())]


TRAIT = re.compile(r"\bpub\s+trait\s+([A-Za-z_]\w*)[^{;]*\{")


def vocab_traits(root):
    """name -> (file, required items, host_area); a name defined twice maps to None."""
    traits = {}
    for path in sorted((root / VOCAB).rglob("*.rs")):
        text = strip(path.read_text())
        for match in TRAIT.finditer(text):
            name = match.group(1)
            before = text[:match.start()].rstrip()
            attributes = re.search(r"((?:#\[(?:[^\[\]]|\[[^\]]*\])*\]\s*)*)$", before).group(1)
            body = text[match.end():block_end(text, match.end() - 1) - 1]
            required = set()
            for item in top_level(body):
                if re.match(r"\s*(?:#\[(?:[^\[\]]|\[[^\]]*\])*\]\s*)*#\[\s*cfg\s*\(", item):
                    continue  # its condition is not evaluated: never required (docstring)
                item = without_attributes(item)
                head = re.match(r"(?:unsafe\s+)?(fn|type|const)\s+([A-Za-z_]\w*)", item)
                if head and item.endswith(";") and not (head.group(1) != "fn" and "=" in item):
                    required.add(head.group(2))
            entry = (path.relative_to(root), required, "host_area" in attributes)
            traits[name] = None if name in traits else entry
    return traits


IMPL = re.compile(r"\bimpl\s*(?:<[^{]*?>)?\s*((?:[A-Za-z_]\w*::)*)([A-Za-z_]\w*)\s*(?:<[^{]*?>)?\s+for\s+"
                  r"([A-Za-z_][\w:]*)(?:<[^{]*?>)?\s*(?:where[^{]*)?\{")


def loopback_files(root, loopback):
    path = root / loopback
    return [path, *sorted((path.parent / path.stem).glob("*.rs"))]


def check_loopback(root, loopback=LOOPBACK):
    files = loopback_files(root, loopback)
    traits = vocab_traits(root)
    members = {union: [] for union in UNIONS}
    for path in sorted((root / PARTS).glob("*.rs")):
        for match in TRAIT.finditer(strip(path.read_text())):
            for union in UNIONS:
                if match.group(1).endswith(union) and match.group(1) != union:
                    members[union].append(match.group(1))
    if not members["CapabilityProviders"]:
        return [f"{PARTS}: no `pub trait *CapabilityProviders` found; the union check would check nothing. If the "
                "provider traits moved, update tools/hotspots/check_stand_ins.py."]
    problems, impls = [], []
    for path in files:
        text = strip(path.read_text())
        rel = path.relative_to(root)
        local = {m.group(1) for m in re.finditer(r"\btrait\s+([A-Za-z_]\w*)", text)}
        for match in IMPL.finditer(text):
            prefix, name, target = match.groups()
            if name in local or prefix not in ("", "mod_api::", "mod_api::prelude::"):
                continue
            body = text[match.end():block_end(text, match.end() - 1) - 1]
            written = {m.group(2) for m in (re.match(r"(?:pub\s+)?(?:unsafe\s+)?(fn|type|const)\s+([A-Za-z_]\w*)",
                                                     without_attributes(i)) for i in top_level(body)) if m}
            impls.append((name, target, written, rel, text.count("\n", 0, match.start()) + 1))
    # Member impls may sit in any file of the loopback: a target name is one type across the test target.
    implemented = {(name, target) for name, target, _, _, _ in impls}
    for name, target, written, rel, line in impls:
        if name in UNIONS:
            for member in members[name]:
                if (member, target) not in implemented:
                    problems.append(f"{rel}:{line}: `{target}` implements the union `{name}`, so it needs "
                                    f"`impl {member} for {target}` (a trait in {PARTS}/): add it to {rel}.")
            continue
        entry = traits.get(name)
        if not entry or entry[2]:
            continue
        source, required, _ = entry
        for item in sorted(required - written):
            problems.append(f"{rel}:{line}: `impl {name} for {target}` lacks `{item}`, required by {source}: "
                            f"add it to that impl in {rel}.")
    return problems


def problems(root, loopback=LOOPBACK):
    missing = missing_inputs(root, loopback)
    if missing:
        return missing
    return check_fixture_files(root) + check_hook_types(root) + check_loopback(root, loopback)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--root", type=Path, default=ROOT)
    ap.add_argument("--loopback", default=LOOPBACK, help="the wire loopback root, relative to --root")
    args = ap.parse_args(argv)
    found = problems(args.root, args.loopback)
    for problem in found:
        print(problem, file=sys.stderr)
    if found:
        print(f"{len(found)} stand-in problem(s); see tools/hotspots/check_stand_ins.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
