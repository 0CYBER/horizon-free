#!/usr/bin/env python3
"""Carry an old-style PR's addition to a hand-kept list into the list's new form, for a rollup worker.

After a switch (tools/hotspots/README.md), a member PR written against the old list conflicts on it in the rollup.
The worker takes main's side of the old file, then runs this: it reads the PR's own diff of the OLD file
(`diff(merge-base(main, head), head)`) and writes the PR's addition in the new form into the working tree. It does
not commit; the worker commits the result as "Carry for #N". Where it cannot place a hunk, it stops before writing
anything and prints the hunk.

Kinds:
- ignored-test: rows of tools/ci/ignored-tests-known-failures.txt go to tools/ci/ignored-tests/<topic>.txt, same
  row format; the topic is --topic or the PR's branch name (from `gh`). A changed or removed row is changed or
  removed where it lives now.
- part-table: a name the PR added to a list of the old crates/horizon-mod-vocab/src/exports/part_table.rs goes to
  the fragment of the parts file that defines it on the PR's head (exports/parts_<file>.rs, found as
  the Parts source parser finds them), in the same scope, visibility and `#[cfg]`, as a `pub use` line
  of its own. A name this tree already exports in that scope with the same cfg, visibility and source is skipped.
  A name it cannot attribute, one exported there differently, or one the PR removed or renamed stops it before
  writing anything, printing the name and its line.
- dispatch-stub: a stand-in the PR added, changed or removed inside `mod stubs` or `mod api` of the old
  dispatch-tests/src/fixtures/shared.rs is written to (or removed from) fixtures/stubs/<Name>.rs or
  fixtures/api/<Name>.rs, as the PR's head has it (with its impls). It stops when that file on this tree is neither
  the PR's base version nor its head version (main changed the same stand-in). Hunks outside the two modules are
  not carried: they still merge into shared.rs.

    python3 tools/hotspots/carry.py ignored-test 3788
    python3 tools/hotspots/carry.py ignored-test 3788 --head pr/3788 --topic attribute-bounds
    python3 tools/hotspots/carry.py dispatch-stub 3911
    python3 tools/hotspots/carry.py part-table 3788
"""
import argparse
from dataclasses import dataclass, field
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_stand_ins  # noqa: E402
import split_part_table as part_table  # noqa: E402


class CarryError(Exception):
    """A carry that cannot be written; the message says why and shows the hunk."""


def git(repo, *args, ok=(0,)):
    p = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    if p.returncode not in ok:
        raise CarryError(f"git {' '.join(args)}: {p.stderr.strip()}")
    return p.returncode, p.stdout


def pr_range(repo, number, head=None, main="origin/main"):
    """(merge base with main, head) of a PR; the head is `refs/pull/<n>/head` unless given."""
    ref = head or f"refs/pull/{number}/head"
    rc, sha = git(repo, "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}", ok=(0, 1))
    if rc:
        raise CarryError(f"{ref} is not fetched: git fetch origin pull/{number}/head:refs/pull/{number}/head, "
                         "or pass --head")
    sha = sha.strip()
    base = git(repo, "merge-base", main, sha)[1].strip()
    return base, sha


# ---- diff reading -------------------------------------------------------------------------------------------------

@dataclass
class Hunk:
    header: str
    removed: list = field(default_factory=list)
    added: list = field(default_factory=list)

    def text(self):
        return "\n".join([self.header] + ["-" + x for x in self.removed] + ["+" + x for x in self.added])


def hunks(repo, base, head, path):
    """The hunks of the PR's diff of one file, without context lines."""
    out = git(repo, "diff", "--no-renames", "-U0", base, head, "--", path)[1]
    found = []
    for line in out.splitlines():
        if line.startswith("@@"):
            found.append(Hunk(line))
        elif found and line.startswith("-") and not line.startswith("---"):
            found[-1].removed.append(line[1:])
        elif found and line.startswith("+") and not line.startswith("+++"):
            found[-1].added.append(line[1:])
    return found


def is_row(line):
    return bool(line.strip()) and not line.lstrip().startswith("#")


# ---- kinds --------------------------------------------------------------------------------------------------------

IGNORED_OLD = "tools/ci/ignored-tests-known-failures.txt"
IGNORED_DIR = "tools/ci/ignored-tests"


def row_key(row):
    return tuple(row.split(" # ", 1)[0].split()[:2])


def carry_ignored_test(repo, base, head, topic):
    """Writes the PR's rows into tools/ci/ignored-tests/<topic>.txt; returns what it did."""
    folder = Path(repo) / IGNORED_DIR
    if not folder.is_dir():
        raise CarryError(f"{IGNORED_DIR}/ does not exist on this tree: the ignored-test switch (PR D) has not landed")
    if not topic:
        raise CarryError("no topic: pass --topic")
    files = {p: p.read_text(encoding="utf-8").splitlines() for p in sorted(folder.glob("*.txt"))}

    def where(key):
        return [(p, i) for p, lines in files.items() for i, x in enumerate(lines) if is_row(x) and row_key(x) == key]

    target = folder / f"{topic}.txt"
    edits, appended, stuck = [], [], []
    found = hunks(repo, base, head, IGNORED_OLD)
    for hunk in found:
        added = {row_key(x): x for x in hunk.added if is_row(x)}
        changes, new_rows, reason = [], [], None
        others = [h for h in found if h is not hunk]
        moved = {row_key(x) for h in others for x in h.added + h.removed if is_row(x)}
        if moved & ({row_key(x) for x in hunk.removed if is_row(x)} | set(added)):
            stuck.append(f"the PR moves a row between hunks:\n{hunk.text()}")
            continue
        for row in filter(is_row, hunk.removed):
            places = where(row_key(row))
            if len(places) != 1:
                reason = f"the removed row is in {len(places)} files under {IGNORED_DIR}/, not one"
                break
            changes.append((*places[0], added.pop(row_key(row), None)))
        for key, row in added.items():
            places = where(key)
            if places and any(files[p][i] != row for p, i in places):
                reason = "an added row's test is already listed with a different row"
            elif not places:
                new_rows.append(row)
        if reason:
            stuck.append(f"{reason}:\n{hunk.text()}")
            continue
        edits += changes
        if new_rows:
            appended += [x for x in hunk.added if x.strip() and not is_row(x)] + new_rows
    if stuck:
        raise CarryError("cannot place these hunks; carry them by hand:\n\n" + "\n\n".join(stuck))
    for path, index, row in edits:
        files[path][index] = row
    if appended:
        files.setdefault(target, [])
        files[target] = files[target] + appended
    done = []
    for path, lines in files.items():
        new = [x for x in lines if x is not None]
        text = "\n".join(new) + "\n" if new else ""
        if not path.exists() or path.read_text(encoding="utf-8") != text:
            if text:
                path.write_text(text, encoding="utf-8")
            else:
                path.unlink()
            done.append(path.relative_to(repo).as_posix())
    return done


SHARED = f"{check_stand_ins.FIXTURES}/shared.rs"
SCOPES = check_stand_ins.FOLDERS


def dedent(text):
    lines = text.split("\n")
    while lines and not lines[0].strip():
        lines.pop(0)
    return "\n".join(re.sub(r"^ {1,4}", "", line) for line in lines).rstrip() + "\n"


def stand_ins(source):
    """{(scope, name): file text} for the stand-ins in `mod stubs` and `mod api` of an old-layout shared.rs, and
    shared.rs without them. A stand-in's text is its item, each impl whose self type it is (or whose trait it is,
    when the self type is not in that module), and a `use` of a path under it, with the comments above each; `use`
    lines of anything else stay in shared.rs."""
    stripped = check_stand_ins.strip(source)
    found, cuts = {}, []
    for scope in SCOPES:
        match = re.search(rf"(?m)^mod\s+{scope}\s*\{{", stripped)
        if not match:
            continue
        close = check_stand_ins.block_end(stripped, match.end() - 1) - 1
        items, previous = [], match.end()
        for start, end in check_stand_ins.spans(stripped, match.end(), close):
            items.append((stripped[start:end], source[previous:end]))
            previous = end
        names = {check_stand_ins.item_name(code) for code, _ in items} - {None}
        groups, kept = {}, []
        for code, text in items:
            name = check_stand_ins.item_name(code)
            if not name:
                body = check_stand_ins.without_attributes(code)
                impl = re.match(r"impl\s*(?:<[^{]*?>)?\s*(?:([\w:]+)(?:<[^{]*?>)?\s+for\s+)?([\w:]+)", body)
                use = re.match(r"(?:pub(?:\([^)]*\))?\s+)?use\s+(\w+)::", body)
                if impl:
                    self_type, trait = impl.group(2).split("::")[-1], (impl.group(1) or "").split("::")[-1]
                    name = self_type if self_type in names else trait
                elif use and use.group(1) in names:
                    name = use.group(1)
            if name in names:
                groups.setdefault(name, []).append(text)
            else:
                kept.append(text)
        for name, texts in groups.items():
            found[(scope, name)] = dedent("".join(texts))
        body = "".join(kept).rstrip()
        replacement = f"mod {scope} {{{body}\n}}" if body.strip() else ""
        cuts.append((match.start(), close + 1, replacement))
    rest = source
    for start, end, replacement in reversed(cuts):
        tail = rest[end:]
        if not replacement:
            rest = rest[:start].rstrip("\n") + "\n" + (tail.lstrip("\n") and "\n" + tail.lstrip("\n"))
        else:
            rest = rest[:start] + replacement + tail
    return found, rest


TOKEN = re.compile(r'''b?r(#*)".*?"\1|b?"(?:\\.|[^"\\])*"|'(?:\\[^']{1,10}|[^\\'\n])'|//[^\n]*|/\*.*?\*/|\w+|\S''',
                   re.S)


def tokens(text):
    """Rust source as tokens: whitespace between tokens does not count, literals and comments are kept whole."""
    return [m.group(0) for m in TOKEN.finditer(text)]


def carry_dispatch_stub(repo, base, head, _topic):
    """Writes the stand-ins the PR changed in the old shared.rs to their files; returns what it did."""
    fixtures = Path(repo) / check_stand_ins.FIXTURES
    if not all((fixtures / scope).is_dir() for scope in SCOPES):
        raise CarryError("fixtures/stubs/ and fixtures/api/ do not exist on this tree: the dispatch-stub switch "
                         "(PR E) has not landed")

    def version(rev):
        rc, text = git(repo, "show", f"{rev}:{SHARED}", ok=(0, 128))
        return stand_ins(text)[0] if rc == 0 else {}

    def same(a, b):
        return (a is None) == (b is None) and tokens(a or "") == tokens(b or "")

    before, after = version(base), version(head)
    writes, stuck = [], []
    for key in sorted(set(before) | set(after)):
        old, new = before.get(key), after.get(key)
        if same(old, new):
            continue
        path = fixtures / key[0] / f"{key[1]}.rs"
        now = path.read_text(encoding="utf-8") if path.exists() else None
        if same(now, new):
            continue
        if not same(now, old):
            stuck.append(f"{path.relative_to(repo).as_posix()} on this tree differs from the PR's base; the PR's "
                         f"head has:\n{new or '(removed)'}")
            continue
        writes.append((path, new))
    if stuck:
        raise CarryError("cannot place these stand-ins; carry them by hand:\n\n" + "\n\n".join(stuck))
    for path, text in writes:
        if text is None:
            path.unlink()
        else:
            path.write_text(text, encoding="utf-8")
    return [path.relative_to(repo).as_posix() for path, _ in writes]


def export_form(attrs, vis, path):
    """An export as compared across fragments: its attributes, visibility and the item it names, with `parts::...`
    and `crate::parts::...` (the two spellings the fragments use) taken as the same path."""
    path = "::".join(path)
    if path == "parts" or path.startswith("parts::"):
        path = "crate::" + path
    return tuple(part_table.attr_text(a) for a in attrs), "".join((vis or "").split()), path


def exported(source):
    """{(scope, bound name): {export form}} of the `use` leaves of an export fragment, at every scope."""
    found = {}

    def walk(items, scope):
        for item in items:
            if item["kind"] == "mod" and item["tokens"][-1] == "}":
                body = item["tokens"]
                walk(part_table.top_items(" ".join(body[body.index("{") + 1:-1])), scope + (item["name"],))
            elif item["kind"] == "use":
                for path, bound in item["leaves"]:
                    found.setdefault((scope, bound), set()).add(export_form(item["attrs"], item["vis"], path))

    walk(part_table.top_items(source), ())
    return found


def carry_part_table(repo, base, head, _topic):
    """Writes the names the PR added to the old part_table.rs into their parts files' fragments; returns what it
    did."""
    exports = Path(repo) / part_table.EXPORTS
    if (Path(repo) / part_table.OLD).exists():
        raise CarryError(f"{part_table.OLD} still exists on this tree: the part-table switch (PR G) has not landed")

    def entries(rev):
        rc, text = git(repo, "show", f"{rev}:{part_table.OLD}", ok=(0, 128))
        return part_table.old_entries(text)[0] if rc == 0 else []

    before, after = entries(base), entries(head)
    added = [e for e in after if e not in before]
    removed = [e for e in before if e not in after]
    with tempfile.TemporaryDirectory() as tree:
        # The PR's own parts files: a name it adds may be defined in a file only its head has.
        for name in git(repo, "ls-tree", "-r", "--name-only", head, "--", part_table.PARTS)[1].splitlines():
            path = Path(tree) / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(git(repo, "show", f"{head}:{name}")[1], encoding="utf-8")
        owners = part_table.owners(tree)
    have = {}
    for path in exports.glob("*.rs"):
        for key, forms in exported(path.read_text(encoding="utf-8")).items():
            have.setdefault(key, set()).update(forms)
    texts, stuck = {}, []

    def line(entry):
        scope, attrs, vis, prefix, name, rename = entry
        return [part_table.attr_text(a) for a in attrs] + [
            f"{vis} use {prefix}::{name}" + (f" as {rename}" if rename else "") + ";"]

    for entry in added:
        scope, name, bound = entry[0], entry[4], entry[5] or entry[4]
        form = export_form(entry[1], entry[2], entry[3].split("::") + [name])
        if form in have.get((scope, bound), ()):
            continue
        if (scope, bound) in have:
            stuck.append(f"{bound} ({'::'.join(scope) or 'root'} scope): this tree already exports that name with "
                         "another cfg, visibility or source; the PR's line is " + " ".join(line(entry)))
            continue
        owner = owners.get(name)
        if owner is None:
            stuck.append(f"{name} ({'::'.join(scope) or 'root'} scope): no parts file on the PR's head defines it "
                         "once; " + " ".join(line(entry)))
            continue
        path = exports / part_table.fragment_name(owner)
        if path not in texts:
            texts[path] = path.read_text(encoding="utf-8") if path.exists() else \
                part_table.render(owner, [], False)
        text, new = texts[path], line(entry)
        if not scope:
            lines = text.split("\n")
            at = next((i for i, x in enumerate(lines) if re.match(r"(?:pub(?:\([^)]*\))? )?mod __private \{", x)),
                      len(lines) - 1)
            texts[path] = "\n".join(lines[:at] + new + lines[at:])
        else:
            if scope != ("__private",):
                stuck.append(f"{name}: scope {'::'.join(scope)} is not one part_table.rs had")
                continue
            block = re.search(r"(?m)^(?:pub(?:\([^)]*\))? )?mod __private \{\n(?:.*\n)*?\}\n", text)
            body = "".join(f"    {x}\n" for x in new)
            if block:
                texts[path] = text[:block.end() - 2] + body + text[block.end() - 2:]
            else:
                texts[path] = text + "mod __private {\n" + body + "}\n"
        have.setdefault((scope, bound), set()).add(form)
    for entry in removed:
        stuck.append(f"{entry[5] or entry[4]} ({'::'.join(entry[0]) or 'root'} scope): the PR removed or changed this "
                     "line of part_table.rs: " + " ".join(line(entry)))
    if stuck:
        raise CarryError("cannot place these names; carry them by hand:\n" + "\n".join(stuck))
    for path, text in texts.items():
        path.write_text(text, encoding="utf-8")
    return [path.relative_to(repo).as_posix() for path in texts]


KINDS = {
    "ignored-test": carry_ignored_test,
    "part-table": carry_part_table,
    "dispatch-stub": carry_dispatch_stub,
}


def topic_from_branch(branch):
    return re.sub(r"[^a-z0-9]+", "-", branch.rsplit("/", 1)[-1].lower()).strip("-")


def branch_of(number):
    p = subprocess.run(["gh", "pr", "view", str(number), "--json", "headRefName", "--jq", ".headRefName"],
                       capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else ""


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("kind", choices=sorted(KINDS))
    ap.add_argument("pr", type=int)
    ap.add_argument("--head", help="the PR's head (default refs/pull/<PR>/head)")
    ap.add_argument("--main", default="origin/main")
    ap.add_argument("--topic", help="ignored-test: the topic file's name (default: from the PR's branch name)")
    ap.add_argument("--repo", default=".")
    args = ap.parse_args(argv)
    run = KINDS[args.kind]
    try:
        repo = Path(git(args.repo, "rev-parse", "--show-toplevel")[1].strip())
        base, head = pr_range(repo, args.pr, args.head, args.main)
        topic = args.topic or (topic_from_branch(branch_of(args.pr)) if args.kind == "ignored-test" else None)
        done = run(repo, base, head, topic)
    except CarryError as e:
        print(f"carry.py {args.kind} #{args.pr}: {e}", file=sys.stderr)
        return 1
    if done:
        print("written (not committed): " + ", ".join(done))
    else:
        print(f"nothing to carry: #{args.pr} adds no row the tree lacks")
    return 0


if __name__ == "__main__":
    sys.exit(main())
