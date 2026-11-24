#!/usr/bin/env python3
"""Check that a change leaves the generated mod API as it was (docs/layers.md#the-mod-api-stays-put).

This tool builds and runs the `hook_snapshot` example of `horizon-mod-api-gen` on the base and on the working tree, then
compares what it wrote: the published loops (loops.json), the game internals a native mod may declare (internals.json),
the hook catalog (catalog.json, hook-notes.md) and the guest's generated hook dispatch. It fails on:
- a difference in loops.json;
- a removed internals.json entry (one becoming `shape_only` too), unless a line of tools/mod_api_removed_internals.txt
  lists it (`path  #issue`, or `path  kind=<kind>  cfg=<cfg>  #issue` for some of a path's entries), and a line that
  lists an entry the head still has; with --allow-added, added and changed entries are listed and pass
  (`cargo horizon-mod update` reports signature-hash changes for declared internals);
- a path a native mod could declare without a kind that gains a second kind (E_INTERNAL_AMBIGUOUS), always;
- any other difference in the generated files, except an added hook's own catalog, note sections and dispatch,
  or an added host call's guest forwarder and generated numbering, with --allow-added. An additive change
  may also make an exact prose revision listed in tools/mod_api_hook_notes_updates.txt.
The injection points' outputs (points.json, points_generated.rs) are never compared: `[[inject]]` and its points are
removed (#19576), and a base that still writes them compares without them. The mod API's types and calls are the surface report's
(tools/ci/surface_report.py), which runs beside this check.

    python3 tools/check_mod_api_unchanged.py                      # the working tree against the merge base with origin/main
    python3 tools/check_mod_api_unchanged.py --base HEAD^1        # against another ref
    python3 tools/check_mod_api_unchanged.py --allow-added        # new hooks and internals are fine
    python3 tools/check_mod_api_unchanged.py --target-dir DIR     # the Cargo build folder of the working tree (default: target)
    python3 tools/check_mod_api_unchanged.py --compare OLD NEW    # compare two snapshot folders, build nothing
    python3 tools/check_mod_api_unchanged.py --snapshots DIR      # use DIR's stored snapshot of the base if it has one
    python3 tools/check_mod_api_unchanged.py --store-snapshot DIR # CI, main's pushes and the merge queue: keep HEAD's

`--store-snapshot` builds and runs the generator on HEAD and keeps its output in DIR/<commit>, beside a marker that
records the key (STORE_FORMAT, the commit, the checkout's `rustc -vV`, HORIZON_POINT_NAMES, RUSTC, RUSTUP_TOOLCHAIN),
the checkout's path and every file's SHA-256, with a digest over all of it; it writes under a temporary name and
renames, and keeps the newest KEEP_SNAPSHOTS. A staged copy (`.horizon-source-staged`) stores nothing. CI passes DIR on
the build folder's sticky disk, which only main's pushes and the merge queue save, so a pull request (whose own copy of
this script runs) can never leave a snapshot there; this script also refuses to store unless SAVE_BUILD_DISK is `true`.
With `--snapshots`, a snapshot whose marker matches its digest and the base's key, and whose files all match their
hashes, stands in for the base's build; anything else is a miss and the base is built as below. The commit decides the
generator's source, its Cargo.lock and the tree it reads; the rest of the key covers what does not come from the commit.

The base is built in a temporary `git worktree`, removed afterwards, into its own build folder (`mod-api-base` inside the
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
DOC = "docs/layers.md#the-mod-api-stays-put"
# The injection points' outputs, removed with `[[inject]]` (#19576): never compared.
POINT_FILES = {"points.json", "points_generated.rs"}
MAX_DIFF_LINES = 120
MAX_LISTED = 40
ROOT_MARK = "<repository>"
REMOVED = "tools/mod_api_removed_internals.txt"
NOTE_UPDATES = "tools/mod_api_hook_notes_updates.txt"
# Files the generator writes into the output folder that are not output: phase timings (and incremental-*/ or, from
# older builds, interfaces-*/, its cache).
SCRATCH_FILES = {"points-timing.txt"}
# Files the head has to write, compared exactly: the published loops and the exposed internals (internals.json passes an
# addition or a change with --allow-added; see internals_diff).
INTERNALS = "internals.json"
STRICT_FILES = {"loops.json", INTERNALS}
HOOK_CATALOG = "catalog.json"
GUEST_WIRE = "wire_guest_generated.rs"
NATIVE_HOOK_RECORDS = "native-hook-records.json"
HOOK_FILES = {HOOK_CATALOG, "hook-notes.md", GUEST_WIRE}
# The guest's `hook` function and its arms, one per export a mod is called through, in the generator's method order.
GUEST_DISPATCH = "pub fn hook(id: u32,"
GUEST_ARM = re.compile(r"^\d+ => \{.*?(?=^\d+ => \{|^_ =>)", re.MULTILINE | re.DOTALL)
GUEST_HOOK_ARM = re.compile(
    r"\d+ => \{ let receiver = (?:"
    r"super::hooks\(\)"
    r"|crate::wire::guest::handler::<dyn crate\s*::\s*(?:Block|BlockEntity|Item|Menu|Fluid|Entity)Part>\(input\.take\(\)\?\)\?"
    r"); "
)
GUEST_HOOK_CALL = re.compile(r"\blet answer = receiver\.([a-zA-Z0-9_]+)\(")
HOOK_NAMES = re.compile(r"\bpub fn hook_name\(id: crate::HookId\) -> &'static str \{ match id \{ ([^\n]*) \} \}")
HOOK_NAME = re.compile(r'crate::HookId::\w+ => "(\w+)",')
WIRE_RECORD = re.compile(r"\b__wire_record_(\d+)\b")
# A guest forwarder's host call: the import's name (with its signature hash) and the generator's position number for it.
HOST_IMPORT = re.compile(r'host_import!\("([^"]+)", \d+u32')
# A guest stub for a host call that has no wasm import (e.g. native-only methods guarded by cfg(not(wasm32))).
GUEST_UNSUPPORTED = re.compile(r'^fn [a-zA-Z0-9_]+.*\{ crate::wire::guest::unsupported\("([^"]+)"\) \}\r?\n\Z')
NATIVE_ONLY_CFG = '#[cfg(not(target_arch = "wasm32"))]'

# The guest implementation of an area trait begins on its own line and each forwarder occupies one line.
GUEST_HOST_IMPL = re.compile(r"^impl (?:\w+\s*::\s*)*\w+ for Guest \{\n", re.MULTILINE)


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
    return {name for name in found if not is_scratch(name)} - POINT_FILES - {NATIVE_HOOK_RECORDS}


def diff_files(base_dir, head_dir, base_root=None, head_root=None):
    """(added, removed, diffs) for the generated files but the points': file names that exist on one side, and
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


def native_hook_records(path):
    """Optional generator provenance, keyed by hook name, with half-open record index intervals."""
    if not path.is_file():
        return {}
    intervals = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(intervals, dict) or any(
            not isinstance(spans, list) or any(
                not isinstance(span, list) or len(span) != 2
                or any(type(index) is not int for index in span) or not 0 <= span[0] <= span[1]
                for span in spans)
            for spans in intervals.values()):
        raise ValueError(f"{NATIVE_HOOK_RECORDS} has invalid index ranges")
    return intervals


def guest_wire_without_hooks(guest, hooks, base, *, added_calls=False, native_records=None):
    """The head's guest wire with the added hooks' names and dispatch arms removed and the generator's private record
    numbers they consumed closed, or None where no such removal turns it into `base`.

    A record number is the generator's index of a named type in the order it first meets them, method by method, and
    the dispatch arms follow that order. Snapshot provenance names the exact numbers first allocated by native-only
    hooks, including Part hooks whose trait methods do not follow global catalog order. Older snapshots only have the
    full `hook_name` table and use the neighboring global arms there. Extra numbers must be explained by an added
    hook's actual allocation or recognized dispatch window; the retained text then has to match the base exactly.

    Owned types are invisible here: an owned event or answer has no record constructor, and an answer's type is not
    even written in its arm. So an owned type an existing hook gains or loses inside an added hook's window is absorbed
    with the added hook's numbers, and the surface report (tools/ci/surface_report.py) is what fails it: a hook that
    gains an output or changes its signature is breaking there. An added hook that is the first to use an owned type a
    later hook already used moves that type's number, so the later numbers shift by less than the ones just after it.
    That fails here: a number missing after the window looks the same as a later hook losing an owned type."""
    catalog = HOOK_NAMES.search(guest)
    for name in hooks:
        guest, count = re.subn(rf'crate::HookId::\w+ => "{re.escape(name)}",', "", guest)
        if count != 1:
            return None
    dispatch = max(guest.find(GUEST_DISPATCH), 0)
    arms = list(GUEST_ARM.finditer(guest, dispatch))
    references = [set(map(int, WIRE_RECORD.findall(arm[0]))) for arm in arms]
    # The host calls' forwarders come before the dispatch and their types are numbered before any hook's, so they
    # bound every window from below, also one ahead of the first retained arm.
    before_dispatch = set(map(int, WIRE_RECORD.findall(guest[:dispatch])))

    retained_arms = {}
    added_names = set()
    removed = []
    for index, arm in enumerate(arms):
        call = GUEST_HOOK_CALL.search(arm[0]) if GUEST_HOOK_ARM.match(arm[0]) else None
        if call and call[1] in hooks:
            added_names.add(call[1])
            removed.append(index)
        elif call:
            retained_arms.setdefault(call[1], set()).update(references[index])
    for index in reversed(removed):
        guest = guest[:arms[index].start()] + guest[arms[index].end():]

    if added_calls:
        # Added imports can allocate records before the hook dispatch. Comparing retained ranks
        # covers both additions while preserving the generated uses of every retained record.
        return records_in_order(guest)

    def window(records, split):
        low = max((n for refs in records[:split] for n in refs), default=-1)
        high = min((n for refs in records[split:] for n in refs if n > low), default=float("inf"))
        return low, high

    removed_indices = set(removed)
    retained = [before_dispatch] + [
        refs if index not in removed_indices else () for index, refs in enumerate(references)]
    windows = [window(retained, index + 1) for index in removed]
    native_records = {} if native_records is None else native_records
    native_hooks = hooks - added_names
    unrecorded = native_hooks - native_records.keys()
    if catalog and unrecorded:
        # Native-only hooks have no dispatch position. Apply the same bounds to the retained arm
        # records in catalog order, skipping other entries that have no guest-side records.
        catalog_records = [before_dispatch]
        splits = set()
        for name in HOOK_NAME.findall(catalog[1]):
            if name in retained_arms:
                catalog_records.append(retained_arms[name])
            elif name in unrecorded:
                splits.add(len(catalog_records))
        windows.extend(window(catalog_records, split) for split in splits)

    head_parts, base_parts = WIRE_RECORD.split(guest), WIRE_RECORD.split(base)
    if len(head_parts) != len(base_parts) or head_parts[::2] != base_parts[::2]:
        return None
    pairs = {}
    for head_number, base_number in zip(map(int, head_parts[1::2]), map(int, base_parts[1::2])):
        if pairs.setdefault(head_number, base_number) != base_number:
            return None
    used = sorted(pairs)
    used_numbers, last_used = set(used), used[-1] if used else 0
    exact_gaps = {
        n for name in native_hooks & native_records.keys()
        for first, end in native_records[name] for n in range(first, min(end, last_used))
        if n not in used_numbers
    }
    gaps = (set().union(*(references[index] for index in removed)) - used_numbers) | exact_gaps
    # Each retained number still above its base number absorbs the unreferenced numbers just below it. Which owned
    # types those were is not in this file: one an existing hook started to use there passes here, and only the
    # surface report sees it. A number the head lacks (a surplus below zero) fails, never absorbed.
    for previous, number in zip([-1] + used, used):
        surplus = number - sum(gap < number for gap in gaps) - pairs[number]
        free = [n for n in range(number - 1, previous, -1) if n not in gaps][:max(surplus, 0)]
        if surplus < 0 or len(free) < surplus:
            return None
        gaps.update(free)
    if not all(any(low < gap < high for low, high in windows) for gap in gaps - exact_gaps):
        return None
    gaps = sorted(gaps)

    def renumber(match):
        number = int(match[1])
        return f"__wire_record_{number - bisect_left(gaps, number)}"

    return WIRE_RECORD.sub(renumber, guest)


def note_sections(page):
    """hook-notes.md as its text before the first `## ` heading, then one string per `## ` section (headings inside a
    code fence don't start one), each without its trailing blank lines: the page puts one before every hook's notes,
    so the last section gains one when a hook is appended after it."""
    sections, fenced = [[]], False
    for line in page.splitlines(keepends=True):
        if line.startswith("```"):
            fenced = not fenced
        if line.startswith("## ") and not fenced:
            sections.append([])
        sections[-1].append(line)
    return ["".join(section).rstrip("\n") for section in sections]


def load_note_updates(path):
    """Read `OLD_SHA256 NEW_SHA256 #ISSUE ## Heading` entries for exact prose changes."""
    revisions = set()
    if not path.is_file():
        return revisions
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        entry = re.fullmatch(r"([0-9a-f]{64})\s+([0-9a-f]{64})\s+#[1-9]\d*\s+(## .+)", line)
        if not entry:
            raise ValueError(f"{path}:{number}: write `OLD_SHA256 NEW_SHA256 #ISSUE ## Heading`")
        old, new, heading = entry.groups()
        key = (heading, old, new)
        if old == new or key in revisions:
            raise ValueError(f"{path}:{number}: repeated or unchanged note revision")
        revisions.add(key)
    return revisions


def notes_only_add_sections(base, head, *, allow_new=True, revisions=()):
    """Keep the preamble and every existing section in order, with only listed exact prose changes."""
    base, head = note_sections(base), note_sections(head)
    if base[0] != head[0] or (not allow_new and len(base) != len(head)):
        return False
    remaining = iter(head[1:])
    revisions = set(revisions)

    def matches(section, other):
        if section == other:
            return True
        heading = section.partition("\n")[0]
        if revisions and heading == other.partition("\n")[0]:
            key = (heading, hashlib.sha256(section.encode("utf-8")).hexdigest(),
                   hashlib.sha256(other.encode("utf-8")).hexdigest())
            if key in revisions:
                revisions.remove(key)
                return True
        return False

    return all(any(matches(section, other) for other in remaining) for section in base[1:])


def guest_wire_without_calls(guest, base):
    """The head's and the base's guest wire with the head's added host calls' forwarders removed and every import's
    position number blanked, and the names of those calls; (None, None, set()) when an added call's line holds another.

    A wasm guest links a host call by its name, which carries the call's signature hash (`Area::method#hash`); the
    number beside it is the generator's position of the call among all imports, used only by a native guest built
    with the game. An added call takes a position, so every later import's number moves by one. A changed or removed
    call loses its base name, and the base's forwarder for it is then missing from the head."""
    known = set(HOST_IMPORT.findall(base))
    known_unsupported = {
        match.group(1)
        for line in base.splitlines(keepends=True)
        if (match := GUEST_UNSUPPORTED.match(line))
    }
    known_impls = set(GUEST_HOST_IMPL.findall(base))
    kept, added = [], set()
    new_impl, removed_forwarder = None, False
    for line in guest.splitlines(keepends=True):
        if GUEST_HOST_IMPL.fullmatch(line):
            new_impl = len(kept) if line not in known_impls else None
            removed_forwarder = False
        names = HOST_IMPORT.findall(line)
        if any(name not in known for name in names):
            if len(names) != 1:
                return None, None, set()
            added.add(names[0])
            if new_impl is not None:
                removed_forwarder = True
            continue
        if not names and (stub := GUEST_UNSUPPORTED.match(line)) and stub.group(1) not in known_unsupported:
            if kept and kept[-1].rstrip("\r\n") == NATIVE_ONLY_CFG:
                kept.pop()
            added.add(stub.group(1))
            if new_impl is not None:
                removed_forwarder = True
            continue
        if line == "}\n" and new_impl is not None:
            # A new area with only added forwarders leaves its empty impl after stripping.
            omit = removed_forwarder and len(kept) == new_impl + 1
            new_impl = None
            if omit:
                kept.pop()
                continue
        kept.append(line)

    def blank(text):
        return HOST_IMPORT.sub(lambda match: f'host_import!("{match[1]}", 0u32', text)

    return blank("".join(kept)), blank(base), added


def records_in_order(guest):
    """The guest wire with each record number replaced by its rank among the file's record numbers.

    A record number is a named type's index in the generator's type table, so a type an added host call brings in
    moves the numbers of the types after it. A head that matches the base this way names the same records in the same
    places, in the same order; which type a record is, is the surface report's (tools/ci/surface_report.py)."""
    rank = {number: index for index, number in enumerate(sorted({int(n) for n in WIRE_RECORD.findall(guest)}))}
    return WIRE_RECORD.sub(lambda match: f"__wire_record_{rank[int(match[1])]}", guest)


def first_difference(head, base):
    """The first line of `head` that differs from `base`, for the report."""
    for new, old in zip(head.splitlines(), base.splitlines()):
        if new != old:
            return new
    return head.splitlines()[len(base.splitlines())] if len(head.splitlines()) > len(base.splitlines()) else "(end)"


def explained_additions(base_dir, head_dir, diffs, base_root, head_root):
    """The catalog hooks and the host calls the head adds, when they explain every generated difference; two empty
    sets when they don't."""
    nothing = set(), set()
    changed = {name for name, _ in diffs}
    if not changed or changed - HOOK_FILES:
        return nothing
    hooks = added_catalog_hooks(base_dir, head_dir) if HOOK_CATALOG in changed else set()
    if HOOK_CATALOG in changed and not hooks:
        return nothing
    if "hook-notes.md" in changed:
        revisions = load_note_updates(Path(head_root or ROOT) / NOTE_UPDATES)
        if not notes_only_add_sections(
                read_text(base_dir / "hook-notes.md", [base_root] if base_root else []),
                read_text(head_dir / "hook-notes.md", [head_root] if head_root else []),
                allow_new=bool(hooks), revisions=revisions):
            return nothing
    calls = set()
    if GUEST_WIRE in changed:
        base = read_text(base_dir / GUEST_WIRE, [base_root] if base_root else [])
        head = read_text(head_dir / GUEST_WIRE, [head_root] if head_root else [])
        head, base, calls = guest_wire_without_calls(head, base)
        if head is None:
            return nothing
        if hooks:
            try:
                native_records = native_hook_records(head_dir / NATIVE_HOOK_RECORDS)
            except ValueError as error:
                print(f"Cannot read native hook record provenance: {error}")
                return nothing
            explained = guest_wire_without_hooks(
                head, hooks, base, added_calls=bool(calls), native_records=native_records)
            if calls:
                base = records_in_order(base)
        elif calls:
            head, base = records_in_order(head), records_in_order(base)
            explained = head
        else:
            explained = None
        if explained != base:
            if explained is not None:
                print(f"{GUEST_WIRE} differs after the added hooks and host calls, first at: "
                      + first_difference(explained, base)[:300])
            return nothing
    return hooks, calls


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


def made_ambiguous(base_text, head_text):
    """The paths a native mod could declare without a kind in the base (one kind among its entries that are not
    `shape_only`, one of them unconditional) that have more than one kind in the head: a kind-less declaration of one
    stops resolving (E_INTERNAL_AMBIGUOUS), as a removal would."""
    def kinds(text):
        out = {}
        for entry in internal_entries(text):
            if not entry.get("shape_only"):
                out.setdefault(entry["path"], {}).setdefault(entry["kind"], set()).add(entry.get("cfg") or "")
        return out

    base, head = kinds(base_text), kinds(head_text)
    return sorted(path for path, by_kind in base.items()
                  if len(by_kind) == 1 and "" in next(iter(by_kind.values())) and len(head.get(path, ())) > 1)


def short(value):
    text = json.dumps(value, ensure_ascii=False) if not isinstance(value, str) else value
    return text if len(text) <= 80 else text[:77] + "..."


LISTING = re.compile(r"(\S+)(?:\s+kind=(\S+))?(?:\s+cfg=(.*?))?\s+(#\d+)")


def load_removed(path):
    """tools/mod_api_removed_internals.txt: listing -> issue. A `path  #issue` line lists every entry of the path, and
    is keyed by the path; `path  kind=<kind>  cfg=<cfg>  #issue`, with either part left out, lists only the entries
    with that kind and cfg (`cfg=` alone: the unconditional ones), keyed by (path, kind, cfg) with None for a part left
    out. Blank lines and lines starting with `#` are comments. A missing file lists nothing."""
    removed = {}
    path = Path(path)
    if not path.is_file():
        return removed
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = LISTING.fullmatch(line.strip())
        # `kind=` after `cfg=` would read as part of the cfg.
        if not match or match.group(3) and re.search(r"(^|\s)kind=", match.group(3)):
            raise ValueError(f"{path}:{number}: write `internal path  #issue` or `internal path  kind=<kind>  "
                             "cfg=<cfg>  #issue`")
        internal, kind, cfg, issue = match.groups()
        key = internal if kind is None and cfg is None else (internal, kind, None if cfg is None else cfg.strip())
        if key in removed:
            raise ValueError(f"{path}:{number}: {listing_name(key)} is listed twice")
        removed[key] = issue
    return removed


def listing_pattern(key):
    """(path, kind, cfg) of a removal listing, None where any matches."""
    return (key, None, None) if isinstance(key, str) else key


def listing_name(key):
    path, kind, cfg = listing_pattern(key)
    return path + (f" kind={kind}" if kind is not None else "") + (f" cfg={cfg}" if cfg is not None else "")


def lists(key, internal):
    """Whether the removal listing `key` covers the internals.json key `internal` (path, kind, cfg)."""
    return all(want is None or want == have for want, have in zip(listing_pattern(key), internal))


def removed_internals(internals, listed, head_keys):
    """(accepted, failing, stale) for internals_diff's removed keys: a removal is accepted when a listing covers it,
    and a listing that covers an entry the head still has is stale, whatever this change removed."""
    removed = internals[1] if internals is not None else []
    accepted = [key for key in removed if any(lists(listing, key) for listing in listed)]
    failing = [key for key in removed if key not in accepted]
    stale = sorted(listing_name(listing) for listing in listed if any(lists(listing, key) for key in head_keys))
    return accepted, failing, stale


def summary(files_added, files_removed, diffs, max_lines=MAX_DIFF_LINES, max_listed=MAX_LISTED, internals=None,
            accepted=(), stale=(), ambiguous=()):
    """The report as a list of lines. `internals` is internals_diff's result, or None when internals.json is the
    same; `accepted` its removals the removal file lists, `stale` the listed paths the head still has."""
    out = []

    def listing(title, names, show):
        if not names:
            return
        out.append(f"{title} ({len(names)}):")
        for name in names[:max_listed]:
            out.append(f"  {show(name)}")
        if len(names) > max_listed:
            out.append(f"  ... and {len(names) - max_listed} more")

    def key(key):
        return f"{key[0]} ({key[1]}" + (f", cfg {key[2]})" if key[2] else ")")

    if internals is not None:
        added, removed, changed = internals
        listing("Internals added", added, key)
        listing(f"Internals removed and listed in {REMOVED} (accepted)", [k for k in removed if k in accepted], key)
        listing("Internals removed and failing", [k for k in removed if k not in accepted], key)
        listing("Internals changed", changed, key)
        listing("Internals made ambiguous without a kind (failing)", list(ambiguous), str)
        if not any(internals):
            out.append("internals.json differs, with the same entries.")
    listing(f"Listed in {REMOVED} but still present", list(stale), str)
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


def verdict(files_added, files_removed, diffs, explained, allow_added=False, internals=None, failing=(), stale=()):
    """0 when the generated API is as it was, or differs only in ways accepted: catalog hooks added (`explained`, with
    allow_added), internals added or changed (with allow_added) and internals removed that the removal file lists. An
    unlisted removal or a stale listing always fails, and so does a loops.json difference."""
    if failing or stale:
        return 1
    if internals is not None and (internals[0] or internals[2]) and not allow_added:
        return 1
    if any(name in STRICT_FILES for name, _ in diffs):
        return 1
    if any(name in STRICT_FILES for name in files_removed):
        return 1
    if (diffs or files_added or files_removed) and not explained:
        return 1
    return 0


def head_internal_keys(head_dir, head_root):
    """The (path, kind, cfg) of every entry of the head's internals.json a native mod can resolve (not
    `shape_only`)."""
    path = Path(head_dir) / INTERNALS
    if not path.is_file():
        return set()
    entries = internal_entries(read_text(path, [head_root] if head_root else []))
    return {(entry["path"], entry["kind"], entry.get("cfg") or "") for entry in entries if not entry.get("shape_only")}


def compare(base_dir, head_dir, allow_added=False, base_root=None, head_root=None, listed=None):
    """Compare two snapshot folders, print the report and return the exit status. `listed` is the removal file's
    entries (default: tools/mod_api_removed_internals.txt)."""
    base_dir, head_dir = Path(base_dir), Path(head_dir)
    files_added, files_removed, diffs = diff_files(base_dir, head_dir, base_root, head_root)
    # A strict file only the head wrote (the base's older hook_snapshot does not write it yet) has nothing to compare.
    # One the base wrote and the head did not always fails (verdict).
    skipped = [n for n in files_added if n in STRICT_FILES]
    files_added = [n for n in files_added if n not in STRICT_FILES]
    for name in skipped:
        print(f"note: {name} exists on one side only, not compared.")
    internals = None
    ambiguous = []
    if any(name == INTERNALS for name, _ in diffs):
        # Listed by entry instead of as a diff: the file is one line of several megabytes. One that does not parse as
        # entries stays a strict file difference, in one line.
        try:
            base_text = read_text(base_dir / INTERNALS, [base_root] if base_root else [])
            head_text = read_text(head_dir / INTERNALS, [head_root] if head_root else [])
            internals = internals_diff(base_text, head_text)
            ambiguous = made_ambiguous(base_text, head_text)
            unreadable = None
        except (ValueError, KeyError, TypeError) as error:
            unreadable = [f"internals.json differs and does not parse as entries ({type(error).__name__}: {error})"]
        diffs = [(name, unreadable if name == INTERNALS else lines) for name, lines in diffs
                 if name != INTERNALS or unreadable]
    if listed is None:
        listed = load_removed(ROOT / REMOVED)
    accepted, failing, stale = removed_internals(internals, listed,
                                                 head_internal_keys(head_dir, head_root) if listed else set())
    explained = False
    # A new named hook or host call changes its generated files; nothing else may.
    if allow_added and not files_added and not files_removed:
        hook_additions, call_additions = explained_additions(base_dir, head_dir, diffs, base_root, head_root)
        if hook_additions:
            print("Catalog hooks added (accepted): " + ", ".join(sorted(hook_additions)))
        if call_additions:
            print("Host calls added (accepted): " + ", ".join(sorted(call_additions)))
        explained = bool(hook_additions or call_additions)
    status = verdict(files_added, files_removed, diffs, explained, allow_added, internals, failing + ambiguous, stale)
    lines = summary(files_added, files_removed, diffs, internals=internals, accepted=accepted, stale=stale,
                    ambiguous=ambiguous)
    if not lines:
        print("Mod API unchanged: the loops, internals, hook catalog and generated dispatch are the same.")
        return 0
    print("\n".join(lines))
    if status:
        print(f"\nMod API check failed: the mod API changed ({DOC}).")
        if failing:
            print(f"An internal a native mod may declare is removed only when {REMOVED} lists its path "
                  "(`path  #issue`).")
        if ambiguous:
            print("A path a native mod could declare without a kind now has several kinds (E_INTERNAL_AMBIGUOUS): "
                  "give the new member another name.")
        if stale:
            print(f"{REMOVED} lists an internal the head still has: remove the line, or the internal.")
        if internals is not None and (internals[0] or internals[2]) and not allow_added:
            print("internals.json may add or change entries only with --allow-added.")
        if any(name in STRICT_FILES for name, _ in diffs) or any(name in STRICT_FILES for name in files_removed):
            print("loops.json has to match the base exactly, and the head has to write it and internals.json.")
        if (diffs or files_added or files_removed) and not explained:
            print("The generated API differs from the base with no added catalog hook or host call to explain it.")
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
    return target_dir / "mod-api-base"


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
        if not isinstance(root, str) or INTERNALS not in files or file_hashes(folder / "out") != files:
            return None, None, "miss: incomplete (its files do not match the marker)"
        return folder / "out", root, "hit"
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        return None, None, f"miss: incomplete ({type(error).__name__})"


def store_head(store, target):
    """--store-snapshot: build the generator on HEAD (as warming the build folder did: a failed build fails), then run
    it and keep the snapshot. Everything after the build only warns: with no snapshot, pull requests build the base as
    before."""
    if not may_store():
        print("Mod API check failed: --store-snapshot runs only where the build folder's disk is saved "
              "(SAVE_BUILD_DISK=true, not a pull request); nothing stored.")
        return 2
    print(f"Build folder {target}, snapshots {store}")
    print(f"Snapshot of HEAD ({ROOT})", flush=True)
    try:
        compiled = cargo(ROOT, target, "working tree", "build")
    except (RuntimeError, OSError) as error:
        print(f"Mod API check failed: {error}")
        return 2
    scratch = None
    try:
        commit = git("rev-parse", "HEAD")
        # A staged copy's generator output is not what a pull request's base worktree, which never has the stamp,
        # would give.
        if (ROOT / STAGED_STAMP).exists():
            raise RuntimeError(f"{STAGED_STAMP} exists, so this checkout is a staged copy")
        rustc = toolchain(ROOT)
        if rustc is None:
            raise RuntimeError("rustc -vV failed, so the snapshot has no key")
        scratch = Path(tempfile.mkdtemp(prefix="mod-api-"))
        ran = cargo(ROOT, target, "working tree", "run", "--", str(scratch / "out"), str(ROOT))
        start = time.monotonic()
        store_snapshot(store, scratch / "out", snapshot_key(commit, rustc), ROOT)
        print(f"Mod API snapshot stored for {commit[:12]}: compile {compiled:.0f} s, generator {ran:.0f} s, "
              f"store {time.monotonic() - start:.1f} s.")
    except (RuntimeError, OSError, ValueError) as error:
        print(f"warning: no mod API snapshot stored (compile {compiled:.0f} s): {error}")
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
                        help="accept added catalog hooks, and added or changed internals")
    parser.add_argument("--target-dir", type=Path,
                        help="Cargo build folder of the working tree; the base builds in mod-api-base inside it (default: "
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
            print(f"Mod API check failed: {error}")
            return 2
    target = resolve_target(args.target_dir, os.environ.get("CARGO_TARGET_DIR"))
    if args.store_snapshot:
        return store_head(args.store_snapshot.resolve(), target)
    scratch = Path(tempfile.mkdtemp(prefix="mod-api-"))
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
        print(f"Mod API base snapshot: {route}; base {base_time}, working tree {head_compile + head_run:.0f} s "
              f"(compile {head_compile:.0f} s, generator {head_run:.0f} s).", flush=True)
        return compare(base_dir, scratch / "head-out", args.allow_added, base_root, ROOT)
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        print(f"Mod API check failed: {error}")
        return 2
    finally:
        if added_worktree:
            subprocess.run(["git", "worktree", "remove", "--force", str(worktree)], cwd=ROOT, capture_output=True)
        subprocess.run(["git", "worktree", "prune"], cwd=ROOT, capture_output=True)
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
