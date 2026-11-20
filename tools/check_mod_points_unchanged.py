#!/usr/bin/env python3
"""Check that a change leaves the mod point catalog and the generated mod API as they were (docs/layers.md#mod-points-stay-put).

Every function under `horizon_client::game::` that the point generator admits is a point (the `&self` and `&mut self`
methods of `Game`, and its free functions and other types' methods), and so is each `#[horizon_mod_api_macros::replaceable]`
function in a lower crate. A point's name, target and signature are part of the mod API. This tool builds and runs the
`hook_snapshot` example of `horizon-mod-api-gen` on the base and on the working tree, then compares what it wrote
(points.json, the generated API, catalog.json, hook-notes.md, loops.json, internals.json). It prints how many points were
added, removed (accepted and failing) and changed, and fails on:
- a removed point unless tools/mod_points_removed.txt lists it (`name | #PR | reason`) and it is not a `Game` method or
  a published loop, and no fixture (tools/ci/fn-replace-check, the wasm host's), doc or mod manifest names it;
- a changed point (name kept, target or hash different) unless the same file lists it;
- an added point or catalog hook, unless --allow-added;
- a difference in loops.json, and a difference in the other generated files that no accepted point change explains (the
  generated API differs whenever a point does). An additive catalog hook can explain its generated catalog and notes
  and its own guest dispatch;
- a removed internals.json entry (one becoming `shape_only` too), and with --allow-added only that: added and changed
  entries are listed and pass (`cargo horizon-mod update` reports signature-hash changes for declared internals).
Moving code out of `game/` removes its points: list them in the file. A method of `Game` keeps its name, module path and
signature, and only its body delegates. A mod that needs a moved function asks for `#[replaceable]` in the lower crate.

    python3 tools/check_mod_points_unchanged.py                      # the working tree against the merge base with origin/main
    python3 tools/check_mod_points_unchanged.py --base HEAD^1        # against another ref
    python3 tools/check_mod_points_unchanged.py --allow-added        # new points are fine
    python3 tools/check_mod_points_unchanged.py --target-dir DIR     # the Cargo build folder of the working tree (default: target)
    python3 tools/check_mod_points_unchanged.py --compare OLD NEW    # compare two snapshot folders, build nothing
    python3 tools/check_mod_points_unchanged.py --snapshots DIR      # use DIR's stored snapshot of the base if it has one
    python3 tools/check_mod_points_unchanged.py --store-snapshot DIR # CI, main's pushes and the merge queue: keep HEAD's

`--store-snapshot` builds and runs the generator on HEAD and keeps its output in DIR/<commit>, beside a marker that
records the key (STORE_FORMAT, the commit, the checkout's `rustc -vV`, HORIZON_POINT_NAMES, RUSTC, RUSTUP_TOOLCHAIN),
the checkout's path and every file's SHA-256, with a digest over all of it; it writes under a temporary name and
renames, and keeps the newest KEEP_SNAPSHOTS. A staged copy (`.horizon-source-staged`) stores nothing. CI passes DIR on
the build folder's sticky disk, which only main's pushes and the merge queue save, so a pull request (whose own copy of
this script runs) can never leave a snapshot there; this script also refuses to store unless SAVE_BUILD_DISK is `true`.
With `--snapshots`, a snapshot whose marker matches its digest and the base's key, and whose files all match their
hashes, stands in for the base's build; anything else is a miss and the base is built as below. The commit decides the
generator's source, its Cargo.lock and the tree it reads; the rest of the key covers what does not come from the commit.

The base is built in a temporary `git worktree`, removed afterwards, into its own build folder (`mod-points-base` inside the
working tree's): Cargo hashes a workspace member's path relative to the workspace root, so the base's and the working
tree's generator would be one unit, and the working tree's older files would make Cargo reuse the base's build. The first
run builds the generator's crates and their dependencies twice.
"""

import argparse
from bisect import bisect_left
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
DOC = "docs/layers.md#mod-points-stay-put"
POINTS = "points.json"
MAX_DIFF_LINES = 120
MAX_LISTED = 40
ROOT_MARK = "<repository>"
LIST = "tools/mod_points_removed.txt"
# Files the generator writes into the output folder that are not output: phase timings (and incremental-*/ or, from
# older builds, interfaces-*/, its cache).
SCRATCH_FILES = {"points-timing.txt"}
# Files the head has to write, compared exactly: the published loops and the exposed internals (internals.json passes an
# addition or a change with --allow-added; see internals_diff).
INTERNALS = "internals.json"
STRICT_FILES = {"loops.json", INTERNALS}
HOOK_CATALOG = "catalog.json"
GUEST_WIRE = "wire_guest_generated.rs"
HOOK_FILES = {HOOK_CATALOG, "hook-notes.md", GUEST_WIRE}
# The guest's `hook` function and its arms, one per export a mod is called through, in the generator's method order.
GUEST_DISPATCH = "pub fn hook(id: u32,"
GUEST_ARM = re.compile(r"^\d+ => \{.*?(?=^\d+ => \{|^_ =>)", re.MULTILINE | re.DOTALL)
GUEST_HOOK_ARM = re.compile(r"\d+ => \{ let receiver = super::hooks\(\); ")
GUEST_HOOK_CALL = re.compile(r"\blet answer = receiver\.([a-zA-Z0-9_]+)\(")
WIRE_RECORD = re.compile(r"\b__wire_record_(\d+)\b")
# Where a mod, a fixture or a doc may name a point.
REFERENCE_PATHS = ("tools/ci/fn-replace-check", "crates/horizon-mod-wasm/tests/fixture", "docs", "*horizon-mod.toml")


def load_points(path):
    """points.json as a dict from point name to its record. A name listed twice is an error: the diff would lose one."""
    with open(path, encoding="utf-8") as f:
        points = json.load(f)
    by_name = {}
    for point in points:
        name = point["name"]
        if name in by_name:
            raise ValueError(f"{path}: point '{name}' is listed twice")
        by_name[name] = point
    return by_name


def diff_points(base, head):
    """(added, removed, changed) between two name-to-record dicts. `changed` holds (name, {field: (old, new)})."""
    added = sorted(set(head) - set(base))
    removed = sorted(set(base) - set(head))
    changed = []
    for name in sorted(set(base) & set(head)):
        fields = {key: (base[name].get(key), head[name].get(key))
                  for key in sorted(set(base[name]) | set(head[name]))
                  if base[name].get(key) != head[name].get(key)}
        if fields:
            changed.append((name, fields))
    return added, removed, changed


def read_text(path, root_marks):
    """A file's text with each source folder's path replaced, so two worktrees' outputs compare."""
    text = path.read_bytes().decode("utf-8", errors="replace")
    for mark in root_marks:
        text = text.replace(str(mark), ROOT_MARK)
    return text


def is_scratch(relative):
    """The generator's own scratch in the output folder: its interface cache (keyed by the generator's executable, so
    two builds never share one) and its phase timings. Older hook_snapshot builds leave them there."""
    parts = Path(relative).parts
    return any(part.startswith(("incremental-", "interfaces-")) for part in parts[:-1]) or parts[-1] in SCRATCH_FILES


def snapshot_files(folder):
    found = {path.relative_to(folder).as_posix() for path in folder.rglob("*") if path.is_file()}
    return {name for name in found if not is_scratch(name)} - {POINTS}


def diff_files(base_dir, head_dir, base_root=None, head_root=None):
    """(added, removed, diffs) for the generated files other than points.json: file names that exist on one side, and
    for each file on both sides that differs, its unified diff as a list of lines."""
    base_dir, head_dir = Path(base_dir), Path(head_dir)
    base_files, head_files = snapshot_files(base_dir), snapshot_files(head_dir)
    diffs = []
    for name in sorted(base_files & head_files):
        old = read_text(base_dir / name, [base_root] if base_root else [])
        new = read_text(head_dir / name, [head_root] if head_root else [])
        if old != new:
            diffs.append((name, list(difflib.unified_diff(
                old.splitlines(), new.splitlines(), f"base/{name}", f"head/{name}", lineterm="", n=1))))
    return sorted(head_files - base_files), sorted(base_files - head_files), diffs


def added_catalog_hooks(base_dir, head_dir):
    base = json.loads((base_dir / HOOK_CATALOG).read_text(encoding="utf-8"))
    head = json.loads((head_dir / HOOK_CATALOG).read_text(encoding="utf-8"))
    try:
        before = {hook["name"] for hook in base["hooks"]}
        after = {hook["name"] for hook in head["hooks"]}
    except (TypeError, KeyError):
        return set()
    if len(before) != len(base["hooks"]) or len(after) != len(head["hooks"]) or not all(
        isinstance(name, str) for name in before | after
    ):
        return set()
    retained = [hook for hook in head["hooks"] if hook["name"] in before]
    return after - before if {**head, "hooks": retained} == base else set()


def guest_wire_without_hooks(guest, hooks, base):
    """The head's guest wire with the added hooks' names and dispatch arms removed and the generator's private record
    numbers they consumed closed, or None where no such removal turns it into `base`.

    A record number is the generator's index of a named type in the order it first meets them, method by method, and
    the dispatch arms follow that order. A new hook's types take the numbers between the arms around its own: those its
    arm references and, for an owned event (which has no record constructor), numbers no arm references. Each number the
    head has more than the base below a retained reference must lie between the records of the arms before and after an
    added one's; the retained text then has to match the base exactly.

    Owned types are invisible here: an owned event or answer has no record constructor, and an answer's type is not
    even written in its arm. So an owned type an existing hook gains or loses inside an added hook's window is absorbed
    with the added hook's numbers, and the surface report (tools/ci/surface_report.py) is what fails it: a hook that
    gains an output or changes its signature is breaking there. An added hook that is the first to use an owned type a
    later hook already used moves that type's number, so the later numbers shift by less than the ones just after it.
    That fails here: a number missing after the window looks the same as a later hook losing an owned type."""
    for name in hooks:
        guest, count = re.subn(rf'crate::HookId::\w+ => "{re.escape(name)}",', "", guest)
        if count != 1:
            return None
    arms = list(GUEST_ARM.finditer(guest, max(guest.find(GUEST_DISPATCH), 0)))
    references = [set(map(int, WIRE_RECORD.findall(arm[0]))) for arm in arms]

    def added(arm):
        call = GUEST_HOOK_CALL.search(arm[0])
        return GUEST_HOOK_ARM.match(arm[0]) is not None and call is not None and call[1] in hooks

    removed = [index for index, arm in enumerate(arms) if added(arm)]
    windows = []
    for index in removed:
        low = max((n for j in range(index) if j not in removed for n in references[j]), default=-1)
        high = min((n for j in range(index + 1, len(arms)) if j not in removed for n in references[j] if n > low),
                   default=float("inf"))
        windows.append((low, high))
    for index in reversed(removed):
        guest = guest[:arms[index].start()] + guest[arms[index].end():]

    head_parts, base_parts = WIRE_RECORD.split(guest), WIRE_RECORD.split(base)
    if len(head_parts) != len(base_parts) or head_parts[::2] != base_parts[::2]:
        return None
    pairs = {}
    for head_number, base_number in zip(map(int, head_parts[1::2]), map(int, base_parts[1::2])):
        if pairs.setdefault(head_number, base_number) != base_number:
            return None
    used = sorted(pairs)
    gaps = set().union(*(references[index] for index in removed)) - set(used)
    # Each retained number still above its base number absorbs the unreferenced numbers just below it. Which owned
    # types those were is not in this file: one an existing hook started to use there passes here, and only the
    # surface report sees it. A number the head lacks (a surplus below zero) fails, never absorbed.
    for previous, number in zip([-1] + used, used):
        surplus = number - sum(gap < number for gap in gaps) - pairs[number]
        free = [n for n in range(number - 1, previous, -1) if n not in gaps][:max(surplus, 0)]
        if surplus < 0 or len(free) < surplus:
            return None
        gaps.update(free)
    if not all(any(low < gap < high for low, high in windows) for gap in gaps):
        return None
    gaps = sorted(gaps)

    def renumber(match):
        number = int(match[1])
        return f"__wire_record_{number - bisect_left(gaps, number)}"

    return WIRE_RECORD.sub(renumber, guest)


def explained_hook_additions(base_dir, head_dir, diffs, base_root, head_root):
    changed = {name for name, _ in diffs}
    if HOOK_CATALOG not in changed or changed - HOOK_FILES:
        return set()
    hooks = added_catalog_hooks(base_dir, head_dir)
    if hooks and GUEST_WIRE in changed:
        base = read_text(base_dir / GUEST_WIRE, [base_root] if base_root else [])
        head = read_text(head_dir / GUEST_WIRE, [head_root] if head_root else [])
        if guest_wire_without_hooks(head, hooks, base) != base:
            return set()
    return hooks


def internal_entries(text):
    """The entries of an internals.json text, each checked to have what the generator always writes (`path`, `kind`,
    `signature`, `hash`) and the optional `cfg` and `shape_only` with their types. A malformed catalog is a ValueError:
    a dropped field would otherwise pass as a change."""
    catalog = json.loads(text)
    entries = catalog.get("internals") if isinstance(catalog, dict) else None
    if not isinstance(entries, list):
        raise ValueError("no `internals` array")
    for number, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise ValueError(f"entry {number} is not an object")
        for field, kind, required in (("path", str, True), ("kind", str, True), ("signature", str, True),
                                      ("hash", str, True), ("cfg", str, False), ("shape_only", bool, False)):
            if (required or field in entry) and not isinstance(entry.get(field), kind):
                raise ValueError(f"entry {number} has no {kind.__name__} `{field}`")
    return entries


def internals_diff(base_text, head_text):
    """(added, removed, changed) between two internals.json texts, each a sorted list of (path, kind, cfg). `path` is
    not unique (a field and a method share one, and `cfg` alternatives repeat both), so entries are keyed as the
    generator writes them apart, with an absent `cfg` as "". A key the head has fewer entries of, or fewer that are not
    `shape_only` (a native mod cannot resolve a shape-only type), counts as removed."""
    def groups(text):
        out = {}
        for entry in internal_entries(text):
            out.setdefault((entry["path"], entry["kind"], entry.get("cfg") or ""), []).append(entry)
        return {key: sorted(json.dumps(entry, sort_keys=True) for entry in entries) for key, entries in out.items()}

    def resolvable(entries):
        return sum(not json.loads(entry).get("shape_only") for entry in entries)

    base, head = groups(base_text), groups(head_text)
    removed = sorted(key for key in base if len(head.get(key, ())) < len(base[key])
                     or resolvable(head.get(key, ())) < resolvable(base[key]))
    added = sorted(set(head) - set(base))
    changed = sorted(key for key in set(base) & set(head) if key not in removed and base[key] != head[key])
    return added, removed, changed


def short(value):
    text = json.dumps(value, ensure_ascii=False) if not isinstance(value, str) else value
    return text if len(text) <= 80 else text[:77] + "..."


def load_accepted(path):
    """tools/mod_points_removed.txt: point name -> (PR, reason). One `name | #PR | reason` line per point; blank lines
    and lines starting with `#` are comments. A missing file lists nothing."""
    accepted = {}
    path = Path(path)
    if not path.is_file():
        return accepted
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = [part.strip() for part in line.split("|", 2)]
        if len(parts) != 3 or not all(parts):
            raise ValueError(f"{path}:{number}: write `point name | #PR | reason`")
        accepted[parts[0]] = (parts[1], parts[2])
    return accepted


def normalized(target):
    return "".join(target.split())


def is_game_method(target):
    """A method of `Game`: the path has `Game` as the type before the method."""
    target = normalized(target)
    return "::Game<'_>::" in target or "::Game::" in target


def loop_targets(loops_json):
    """The entry and generic functions of the published loops in a loops.json, normalized, or None when there is no
    such file (a base built by an older hook_snapshot): the loops are then unknown, not absent."""
    if not loops_json.is_file():
        return None
    out = set()
    for entry in json.loads(loops_json.read_text(encoding="utf-8")).get("loops", []):
        out.add(normalized(entry["entry"]))
        owner = entry["entry"].rsplit("::", 1)[0]
        out.add(normalized(f"{owner}::{entry['generic']}"))
    return out


def references(point, files):
    """The first checked-in file that names the point (its name as a word, or its target), or None."""
    word = re.compile(r"(?<![\w:])" + re.escape(point["name"]) + r"(?!\w)")
    target = re.compile(r"(?<![\w:])" + re.escape(point["target"]) + r"(?!\w)")
    for path in sorted(files):
        text = files[path]
        if target.search(text) or word.search(text):
            return path
    return None


def repo_texts(root=ROOT):
    """The checked-in files a mod may name a point in: the fn-replace fixtures, the wasm host's fixture, the docs and
    every mod manifest, as {path: text}."""
    listed = subprocess.run(["git", "ls-files", "-z", "--", *REFERENCE_PATHS], cwd=root, capture_output=True, text=True)
    if listed.returncode != 0:
        # An empty map would let a removal pass that a fixture names.
        raise RuntimeError(f"git ls-files failed (exit {listed.returncode}): {listed.stderr.strip()}")
    files = {}
    for name in filter(None, listed.stdout.split("\0")):
        try:
            files[name] = (Path(root) / name).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
    return files


def classify(removed, changed, base_points, accepted, loops, files):
    """Split the removed and changed points into accepted and failing: {name: reason}. A removal is accepted when it is
    listed in the removal file and is neither a `Game` method, a loop (`loops` is None when the base's loops are
    unknown: then no removal passes), nor named by a fixture, doc or mod manifest. A change is accepted when it is
    listed."""
    removed_ok, removed_bad, changed_ok, changed_bad = {}, {}, {}, {}
    for name in removed:
        point = base_points[name]
        reasons = []
        if is_game_method(point["target"]):
            reasons.append("a `Game` method")
        if loops is None:
            reasons.append("the base has no loops.json, so a published loop cannot be ruled out")
        elif normalized(point["target"]) in loops:
            reasons.append("a published loop")
        if (path := references(point, files)) is not None:
            reasons.append(f"named in {path}")
        if reasons:
            removed_bad[name] = ", ".join(reasons)
        elif name not in accepted:
            removed_bad[name] = f"not listed in {LIST}"
        else:
            removed_ok[name] = accepted[name]
    for name, _fields in changed:
        if name in accepted:
            changed_ok[name] = accepted[name]
        else:
            changed_bad[name] = f"not listed in {LIST}"
    return removed_ok, removed_bad, changed_ok, changed_bad


def summary(added, removed_ok, removed_bad, changed, changed_ok, files_added, files_removed, diffs, base_points,
            head_points, max_lines=MAX_DIFF_LINES, max_listed=MAX_LISTED, internals=None):
    """The report as a list of lines. `internals` is internals_diff's result, or None when internals.json is the
    same."""
    out = []

    def listing(title, names, show):
        if not names:
            return
        out.append(f"{title} ({len(names)}):")
        for name in names[:max_listed]:
            out.append(f"  {show(name)}")
        if len(names) > max_listed:
            out.append(f"  ... and {len(names) - max_listed} more")

    def old(n):
        return f"{n}  target {base_points[n]['target']}  hash {base_points[n]['signature_hash']}"

    listing("Points added", added,
            lambda n: f"{n}  target {head_points[n]['target']}  hash {head_points[n]['signature_hash']}")
    listing("Points removed and listed (accepted)", sorted(removed_ok),
            lambda n: f"{old(n)}  [{removed_ok[n][0]}: {removed_ok[n][1]}]")
    listing("Points removed and failing", sorted(removed_bad), lambda n: f"{old(n)}  [{removed_bad[n]}]")
    if changed:
        out.append(f"Points changed ({len(changed)}):")
        for name, fields in changed[:max_listed]:
            out.append(f"  {name}" + (f"  [accepted, {changed_ok[name][0]}: {changed_ok[name][1]}]"
                                       if name in changed_ok else ""))
            for key, (before, after) in fields.items():
                out.append(f"    {key}: {short(before)} -> {short(after)}")
        if len(changed) > max_listed:
            out.append(f"  ... and {len(changed) - max_listed} more")
    if internals is not None:
        for title, keys in zip(("added", "removed", "changed"), internals):
            listing(f"Internals {title}", keys,
                    lambda key: f"{key[0]} ({key[1]}" + (f", cfg {key[2]})" if key[2] else ")"))
        if not any(internals):
            out.append("internals.json differs, with the same entries.")
    listing("Generated files added", files_added, str)
    listing("Generated files removed", files_removed, str)
    if diffs:
        out.append(f"Generated files changed ({len(diffs)}): " + ", ".join(name for name, _ in diffs[:max_listed]))
        left = max_lines
        for name, lines in diffs:
            if left <= 0:
                out.append("... more diff lines left out")
                break
            out.extend(lines[:left])
            if len(lines) > left:
                out.append(f"... {len(lines) - left} more lines of {name} left out")
            left -= len(lines)
    return out


def verdict(added, removed_bad, changed_bad, files_added, files_removed, diffs, explained, allow_added=False,
            internals=None):
    """0 when the catalog and the API are as they were, or differ only in ways accepted: points added (with
    allow_added), removals and changes the removal file lists. A failing removal or change always fails. The generated
    API differs whenever a point does, so those diffs pass when `explained` (a point difference was accepted), except
    loops.json, which always has to match. `internals` (internals_diff's result, internals.json left out of `diffs`)
    passes with allow_added when it removes nothing, and never explains another file's diff."""
    if removed_bad or changed_bad or (added and not allow_added):
        return 1
    if internals is not None and (internals[1] or not allow_added):
        return 1
    if any(name in STRICT_FILES for name, _ in diffs):
        return 1
    if any(name in STRICT_FILES for name in files_removed):
        return 1
    if (diffs or files_added or files_removed) and not explained:
        return 1
    return 0


def compare(base_dir, head_dir, allow_added=False, base_root=None, head_root=None, accepted=None, files=None):
    """Compare two snapshot folders, print the report and return the exit status. `accepted` is the removal file's
    entries (default: tools/mod_points_removed.txt) and `files` a function giving the checked-in files a point may be
    named in (default: the repository's)."""
    base_dir, head_dir = Path(base_dir), Path(head_dir)
    base_points, head_points = load_points(base_dir / POINTS), load_points(head_dir / POINTS)
    added, removed, changed = diff_points(base_points, head_points)
    files_added, files_removed, diffs = diff_files(base_dir, head_dir, base_root, head_root)
    # A strict file only the head wrote (the base's older hook_snapshot does not write it yet) has nothing to compare.
    # One the base wrote and the head did not always fails (verdict).
    skipped = [n for n in files_added if n in STRICT_FILES]
    files_added = [n for n in files_added if n not in STRICT_FILES]
    for name in skipped:
        print(f"note: {name} exists on one side only, not compared.")
    internals = None
    if any(name == INTERNALS for name, _ in diffs):
        # Listed by entry instead of as a diff: the file is one line of several megabytes. One that does not parse as
        # entries stays a strict file difference, in one line.
        try:
            internals = internals_diff(read_text(base_dir / INTERNALS, [base_root] if base_root else []),
                                       read_text(head_dir / INTERNALS, [head_root] if head_root else []))
            unreadable = None
        except (ValueError, KeyError, TypeError) as error:
            unreadable = [f"internals.json differs and does not parse as entries ({type(error).__name__}: {error})"]
        diffs = [(name, unreadable if name == INTERNALS else lines) for name, lines in diffs
                 if name != INTERNALS or unreadable]
    if removed and files is None:
        files = repo_texts
    if accepted is None:
        accepted = load_accepted(ROOT / LIST)
    removed_ok, removed_bad, changed_ok, changed_bad = classify(
        removed, changed, base_points, accepted, loop_targets(base_dir / "loops.json"), files() if removed else {})
    explained = bool((added and allow_added) or removed_ok or changed_ok)
    # Function replacement points and named hooks are produced from different inputs. A new
    # named hook may change its generated files while leaving every replacement point intact.
    if allow_added and not explained and not files_added and not files_removed:
        hook_additions = explained_hook_additions(base_dir, head_dir, diffs, base_root, head_root)
        if hook_additions:
            print("Catalog hooks added (accepted): " + ", ".join(sorted(hook_additions)))
            explained = True
    status = verdict(added, removed_bad, changed_bad, files_added, files_removed, diffs, explained, allow_added,
                     internals)
    lines = summary(added, removed_ok, removed_bad, changed, changed_ok, files_added, files_removed, diffs,
                    base_points, head_points, internals=internals)
    counts = (f"Mod points: {len(added)} added, {len(removed)} removed ({len(removed_ok)} accepted, "
              f"{len(removed_bad)} failing), {len(changed)} changed ({len(changed_ok)} accepted, "
              f"{len(changed_bad)} failing).")
    if not lines:
        print(f"Mod points unchanged: {len(head_points)} points, the generated API is the same.")
        print(counts)
        return 0
    print("\n".join(lines))
    print("\n" + counts)
    if status:
        print(f"Mod point check failed: the mod API changed ({DOC}).")
        if added and not allow_added:
            print("Added points fail without --allow-added.")
        if removed_bad or changed_bad:
            print(f"A removed or changed point passes only when it is listed in {LIST} (name | #PR | reason), is not a "
                  "`Game` method or a loop, and no fixture, doc or mod manifest names it. A mod that needs one "
                  "asks for `#[horizon_mod_api_macros::replaceable]` in the lower crate.")
        if any(name in STRICT_FILES for name, _ in diffs) or any(name in STRICT_FILES for name in files_removed):
            print("loops.json has to match the base exactly, and the head has to write it and internals.json.")
        if internals is not None and (internals[1] or not allow_added):
            print("internals.json may add or change entries with --allow-added, never remove one.")
        if (diffs or files_added or files_removed) and not explained:
            print("The generated API differs from the base with no accepted point change.")
    return status


def git(*args, cwd=ROOT):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {result.stderr.strip() or result.stdout.strip()}")
    return result.stdout.strip()


def default_base():
    try:
        return git("merge-base", "HEAD", "origin/main")
    except RuntimeError as error:
        raise RuntimeError(f"{error}\nNo origin/main to compare with: pass --base REF.")


LOCK_OUT_OF_DATE = "cannot update the lock file"
TAIL_LINES = 30


def base_target(target_dir):
    """The base's own Cargo build folder, inside the working tree's. A shared one makes Cargo see the working tree's
    generator, whose files are older than the base's build, as built already, and run the base's generator on the
    working tree's declarations."""
    return target_dir / "mod-points-base"


def cargo(root, target_dir, side, verb, *extra):
    """`cargo build` or `cargo run` of the hook_snapshot example in the tree at `root`, quiet, and the seconds it took.
    Cargo's and the example's output is captured: nothing on success, the end of stderr on failure."""
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir))
    command = ["cargo", verb, "--locked", "--quiet", "-p", "horizon-mod-api-gen", "--example", "hook_snapshot", *extra]
    start = time.monotonic()
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True, errors="replace")
    if result.returncode == 0:
        return time.monotonic() - start
    stderr = result.stderr.strip()
    if LOCK_OUT_OF_DATE in stderr:
        if side == "base":
            raise RuntimeError("the base's Cargo.lock is out of date: pass --base with a revision whose Cargo.lock is current")
        raise RuntimeError(f"the {side}'s Cargo.lock is out of date: run cargo once without --locked, then rerun")
    tail = "\n".join(stderr.splitlines()[-TAIL_LINES:])
    raise RuntimeError(f"hook_snapshot failed on the {side} (exit {result.returncode}): {' '.join(command)}\n{tail}")


def snapshot(root, out, target_dir, side):
    """Build, then run, the hook_snapshot example of the tree at `root`, writing into `out`: (compile, generator)
    seconds. Built first so the log tells the two apart."""
    print(f"Snapshot of the {side} ({root})", flush=True)
    compiled = cargo(root, target_dir, side, "build")
    ran = cargo(root, target_dir, side, "run", "--", str(out), str(root))
    return compiled, ran


# A stored snapshot: STORE/<commit>/out (what hook_snapshot wrote) and STORE/<commit>/marker.json, outside the compared
# folder. A change to this layout or to what decides a snapshot raises STORE_FORMAT, so older snapshots miss.
STORE_FORMAT = 1
MARKER = "marker.json"
KEEP_SNAPSHOTS = 30
TEMPORARY = ".partial-"


STAGED_STAMP = ".horizon-source-staged"


def toolchain(root):
    """`rustc -vV` of the compiler Cargo uses in the checkout at `root` (`RUSTC`, else rustup's choice there, a
    directory override included), or None when it cannot run. It is the base's compiler only when the checkout selects
    the toolchain as the base does (selects_base_toolchain)."""
    try:
        result = subprocess.run([os.environ.get("RUSTC") or "rustc", "-vV"], cwd=root, capture_output=True, text=True)
    except OSError:
        return None
    return result.stdout.strip() if result.returncode == 0 and result.stdout.strip() else None


# The files in a directory that rustup reads to choose its toolchain.
TOOLCHAIN_FILES = ("rust-toolchain", "rust-toolchain.toml")


def selects_base_toolchain(base, root=ROOT):
    """Whether the checkout at `root` has the base's toolchain files (the same text, or both without the file), so the
    compiler probed in the checkout is the one a build of the base would get. A file the change adds, edits or removes,
    tracked or not, makes this False."""
    for name in TOOLCHAIN_FILES:
        in_base = git("show", f"{base}:{name}", cwd=root) if git("ls-tree", "--name-only", base, "--", name,
                                                                   cwd=root) else None
        path = Path(root) / name
        here = path.read_text(encoding="utf-8", errors="replace").strip() if path.is_file() else None
        if in_base != here:
            return False
    return True


def snapshot_key(commit, rustc, env=None):
    """Everything that decides a snapshot besides the commit's tree: the stored and the looked-up key must be equal."""
    env = os.environ if env is None else env
    return {"format": STORE_FORMAT, "commit": commit, "rustc": rustc, "point_names": env.get("HORIZON_POINT_NAMES"),
            "rustc_env": env.get("RUSTC"), "rustup_toolchain": env.get("RUSTUP_TOOLCHAIN")}


def file_hashes(folder):
    """{relative path: SHA-256} of every file under `folder`."""
    folder = Path(folder)
    return {path.relative_to(folder).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(folder.rglob("*")) if path.is_file()}


def marker_digest(key, root, files):
    """SHA-256 over everything the marker says, so a damaged key, root or file list is a miss."""
    text = json.dumps({"key": key, "root": root, "files": files}, sort_keys=True)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def may_store(env=None):
    """Only where the disk is saved (SAVE_BUILD_DISK: main's pushes and the merge queue), never in a pull request."""
    env = os.environ if env is None else env
    return env.get("SAVE_BUILD_DISK") == "true" and env.get("GITHUB_EVENT_NAME") != "pull_request"


def store_snapshot(store, out, key, root, keep=KEEP_SNAPSHOTS):
    """Keep the snapshot folder `out`, made from the tree at `root`, as STORE/<commit>: copied under a temporary name
    with its marker, then renamed. The marker records the generating root, which the comparison replaces in the text as
    it replaces a base worktree's. Then keeps the newest `keep` snapshots. A failed store leaves no partial copy: on a
    full disk it would take the space the job's next build needs."""
    store = Path(store)
    store.mkdir(parents=True, exist_ok=True)
    commit = key["commit"]
    partial = store / f"{TEMPORARY}{commit}-{os.getpid()}"
    try:
        shutil.rmtree(partial, ignore_errors=True)
        shutil.copytree(out, partial / "out")
        files = file_hashes(partial / "out")
        marker = {"key": key, "root": str(root), "files": files, "digest": marker_digest(key, str(root), files)}
        (partial / MARKER).write_text(json.dumps(marker, sort_keys=True), encoding="utf-8")
        final = store / commit
        shutil.rmtree(final, ignore_errors=True)
        os.replace(partial, final)
    except BaseException:
        shutil.rmtree(partial, ignore_errors=True)
        raise
    prune(store, keep)


def prune(store, keep=KEEP_SNAPSHOTS):
    """Remove every snapshot but the newest `keep` (by marker time), and leftovers of an interrupted store."""
    kept = []
    for entry in Path(store).iterdir():
        if not entry.is_dir():
            continue
        marker = entry / MARKER
        if entry.name.startswith(TEMPORARY) or not marker.is_file():
            shutil.rmtree(entry, ignore_errors=True)
        else:
            kept.append((marker.stat().st_mtime, entry.name, entry))
    for _, _, entry in sorted(kept, reverse=True)[keep:]:
        shutil.rmtree(entry, ignore_errors=True)


def find_snapshot(store, key):
    """(folder, root, route) for the stored snapshot of key's commit: the snapshot folder and the root it was made from
    on a hit, (None, None, why) on a miss. Anything that is not a complete snapshot under exactly this key is a miss,
    never an error."""
    folder = Path(store) / key["commit"]
    try:
        if not folder.is_dir():
            return None, None, "miss: none stored"
        marker = json.loads((folder / MARKER).read_text(encoding="utf-8"))
        stored, root, files = marker["key"], marker["root"], marker["files"]
        if marker["digest"] != marker_digest(stored, root, files):
            return None, None, "miss: incomplete (the marker does not match its digest)"
        if stored.get("commit") != key["commit"]:
            return None, None, "miss: incomplete (the marker names another commit)"
        if stored != key:
            differs = ", ".join(name for name in sorted(set(stored) | set(key)) if stored.get(name) != key.get(name))
            return None, None, f"miss: stored under another key ({differs})"
        if not isinstance(root, str) or POINTS not in files or file_hashes(folder / "out") != files:
            return None, None, "miss: incomplete (its files do not match the marker)"
        return folder / "out", root, "hit"
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        return None, None, f"miss: incomplete ({type(error).__name__})"


def store_head(store, target):
    """--store-snapshot: build the generator on HEAD (as warming the build folder did: a failed build fails), then run
    it and keep the snapshot. Everything after the build only warns: with no snapshot, pull requests build the base as
    before."""
    if not may_store():
        print("Mod point check failed: --store-snapshot runs only where the build folder's disk is saved "
              "(SAVE_BUILD_DISK=true, not a pull request); nothing stored.")
        return 2
    print(f"Build folder {target}, snapshots {store}")
    print(f"Snapshot of HEAD ({ROOT})", flush=True)
    try:
        compiled = cargo(ROOT, target, "working tree", "build")
    except (RuntimeError, OSError) as error:
        print(f"Mod point check failed: {error}")
        return 2
    scratch = None
    try:
        commit = git("rev-parse", "HEAD")
        # A staged copy's generator compiles the points' sites in (STAGED in points_generated.rs): not what a pull
        # request's base worktree, which never has the stamp, would give.
        if (ROOT / STAGED_STAMP).exists():
            raise RuntimeError(f"{STAGED_STAMP} exists, so this checkout is a staged copy")
        rustc = toolchain(ROOT)
        if rustc is None:
            raise RuntimeError("rustc -vV failed, so the snapshot has no key")
        scratch = Path(tempfile.mkdtemp(prefix="mod-points-"))
        ran = cargo(ROOT, target, "working tree", "run", "--", str(scratch / "out"), str(ROOT))
        start = time.monotonic()
        store_snapshot(store, scratch / "out", snapshot_key(commit, rustc), ROOT)
        print(f"Mod points snapshot stored for {commit[:12]}: compile {compiled:.0f} s, generator {ran:.0f} s, "
              f"store {time.monotonic() - start:.1f} s.")
    except (RuntimeError, OSError, ValueError) as error:
        print(f"warning: no mod points snapshot stored (compile {compiled:.0f} s): {error}")
    finally:
        if scratch is not None:
            shutil.rmtree(scratch, ignore_errors=True)
    return 0


def resolve_target(option, env_value):
    """The one absolute Cargo build folder of the working tree (the base's is inside it): Cargo reads a relative one
    against its working directory, and the two runs have different ones."""
    return Path(option or env_value or ROOT / "target").resolve()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="the git ref to compare with (default: the merge base of HEAD and origin/main)")
    parser.add_argument("--allow-added", action="store_true",
                        help="accept added points and catalog hooks, and added or changed internals")
    parser.add_argument("--target-dir", type=Path,
                        help="Cargo build folder of the working tree; the base builds in mod-points-base inside it (default: "
                             "$CARGO_TARGET_DIR, else the repository's target)")
    parser.add_argument("--compare", nargs=2, metavar=("OLD", "NEW"), type=Path,
                        help="compare two snapshot folders instead of building")
    parser.add_argument("--snapshots", type=Path, metavar="DIR",
                        help="stored snapshots: the base's, when stored complete under the same key, replaces its build")
    parser.add_argument("--store-snapshot", type=Path, metavar="DIR",
                        help="build and run the generator on HEAD and keep its snapshot in DIR, compare nothing (CI: "
                             "main's pushes and the merge queue only)")
    args = parser.parse_args(argv)
    if args.compare:
        try:
            return compare(args.compare[0], args.compare[1], args.allow_added)
        except (OSError, ValueError, KeyError, RuntimeError) as error:
            print(f"Mod point check failed: {error}")
            return 2
    target = resolve_target(args.target_dir, os.environ.get("CARGO_TARGET_DIR"))
    if args.store_snapshot:
        return store_head(args.store_snapshot.resolve(), target)
    scratch = Path(tempfile.mkdtemp(prefix="mod-points-"))
    worktree = scratch / "base"
    added_worktree = False
    try:
        base = git("rev-parse", args.base) if args.base else default_base()
        print(f"Base {base[:12]}, build folder {target}")
        base_dir, base_root, route = None, None, "built: no snapshot folder given"
        if args.snapshots:
            try:
                same_toolchain = selects_base_toolchain(base, ROOT)
            except (RuntimeError, OSError):
                same_toolchain = None
            if same_toolchain is None:
                route = "miss: the toolchain files could not be read"
            elif not same_toolchain:
                route = "miss: the change selects another toolchain"
            elif (rustc := toolchain(ROOT)) is None:
                route = "miss: rustc -vV failed"
            else:
                base_dir, base_root, route = find_snapshot(args.snapshots, snapshot_key(base, rustc))
        if base_dir is None:
            git("worktree", "add", "--detach", "--quiet", str(worktree), base)
            added_worktree = True
            base_dir, base_root = scratch / "base-out", worktree
            base_compile, base_run = snapshot(worktree, base_dir, base_target(target), "base")
            base_time = f"{base_compile + base_run:.0f} s (compile {base_compile:.0f} s, generator {base_run:.0f} s)"
        else:
            base_time = "0 s (stored)"
        head_compile, head_run = snapshot(ROOT, scratch / "head-out", target, "working tree")
        print(f"Mod points base snapshot: {route}; base {base_time}, working tree {head_compile + head_run:.0f} s "
              f"(compile {head_compile:.0f} s, generator {head_run:.0f} s).", flush=True)
        return compare(base_dir, scratch / "head-out", args.allow_added, base_root, ROOT)
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        print(f"Mod point check failed: {error}")
        return 2
    finally:
        if added_worktree:
            subprocess.run(["git", "worktree", "remove", "--force", str(worktree)], cwd=ROOT, capture_output=True)
        subprocess.run(["git", "worktree", "prune"], cwd=ROOT, capture_output=True)
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
