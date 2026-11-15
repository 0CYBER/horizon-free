#!/usr/bin/env python3
"""Keep the conflict hotspots that are already fixed from coming back (AGENTS.md, "No conflict hotspots").

Each guard compares this change with the merge base, the way tools/merge_base.py does, so what is on main
today stays and only growth fails:
- docs/modding/src/SUMMARY.md may not gain a line for a page the book adds on its own
  (tools/modding_concept_pages.py: a page in concepts/ or how-to/, or in a folder beside its parent page);
- a change may touch at most 5 files under launcher/lang/: English is the `tr()` text and a missing translation
  falls back to it (launcher/src/i18n/tests.rs). A rollup or an upstream sync carries other PRs' changes and is not
  checked; it is recognised by its `rollup` or `sync` label, or by the branch name our rollups and syncs use
  (CI passes the branch only for a pull request from this repository, so a fork cannot borrow the name);
- crates/horizon-mod-vocab/src/hooks/notes.rs may not grow.

    python3 tools/hotspots/check_guards.py --base origin/main [--branch NAME] [--labels "a b"]
"""

import argparse
import copy
from pathlib import Path
import re
import subprocess
import sys

TOOLS = Path(__file__).resolve().parents[1]
sys.path.append(str(TOOLS))
import merge_base  # noqa: E402
import modding_concept_pages as pages  # noqa: E402

ROOT = TOOLS.parent
BOOK_SRC = "docs/modding/src"
SUMMARY = f"{BOOK_SRC}/SUMMARY.md"
LANG = "launcher/lang/"
LANG_LIMIT = 5
NOTES = "crates/horizon-mod-vocab/src/hooks/notes.rs"
# A chapter line, with an optional trailing HTML comment (mdBook still renders the chapter).
SUMMARY_LINE = re.compile(r"^(\s*)(?:[-*] )?\[([^\]]*)\]\(([^)]*)\)\s*(?:<!--.*-->\s*)?$")
# The labels and branch names of rollup and upstream-sync pull requests (AGENTS.md, handbook "Rollups"):
# fork-review-rollup-<n>, rollup-<n>, sync-<n>, sync-after-<x>, upstream-sync-<n>.
CARRIER_LABELS = {"rollup", "sync"}
CARRIER_BRANCH = re.compile(r"(?:fork-review-rollup|rollup|upstream-sync|sync)-.+")


def git(root, *args, check=True):
    return subprocess.run(["git", *args], cwd=root, capture_output=True, encoding="utf-8", check=check).stdout


def summary_book(text, src):
    """SUMMARY.md as mdBook's book JSON: parts and (nested) chapters, comments skipped."""
    sections, stack, comment = [], [], False
    for line in text.splitlines()[1:]:  # the first line is the book's own `# Summary`
        if comment or line.lstrip().startswith("<!--"):
            comment = "-->" not in line
            continue
        if line.startswith("# "):
            sections.append({"PartTitle": line[2:].strip()})
            stack = []
            continue
        match = SUMMARY_LINE.match(line)
        if not match:
            continue
        indent, name, path = len(match[1]), match[2], match[3] or None
        if path and (src / path).is_file():
            chapter = pages.chapter(src, path, [], [])
        else:
            chapter = dict(name=name, content="", path=path, source_path=path, number=[], parent_names=[],
                           sub_items=[])
        while stack and stack[-1][0] >= indent:
            stack.pop()
        (stack[-1][1]["sub_items"] if stack else sections).append({"Chapter": chapter})
        stack.append((indent, chapter))
    return {"sections": sections}


def rendered_paths(book, src):
    """The pages the book renders once the preprocessor has added the ones nobody listed."""
    pages.discover(book, src)
    for chapter in list(pages.chapters(book["sections"])):
        if chapter["path"] and (src / chapter["path"]).is_file():
            pages.nest(chapter, src)
    return {chapter["path"] for chapter in pages.chapters(book["sections"]) if chapter["path"]}


def without(items, path):
    """`items` with the chapter at `path` taken out and its listed sub-chapters left in its place."""
    out = []
    for item in items:
        chapter = item.get("Chapter")
        if chapter and chapter["path"] == path:
            out.extend(chapter["sub_items"])
            continue
        if chapter:
            chapter["sub_items"] = without(chapter["sub_items"], path)
        out.append(item)
    return out


def redundant_summary_lines(text, src):
    """The pages SUMMARY.md lists that the book would render without their line."""
    book = summary_book(text, src)
    listed = [c["path"] for c in pages.chapters(book["sections"]) if c["path"] and (src / c["path"]).is_file()]
    redundant = []
    for path in listed:
        trial = copy.deepcopy(book)
        trial["sections"] = without(trial["sections"], path)
        if path in rendered_paths(trial, src):
            redundant.append(path)
    return redundant


def listed_paths(text):
    """{link title: path} of SUMMARY.md's chapter lines."""
    return {m[2]: m[3] for m in map(SUMMARY_LINE.match, text.splitlines()) if m and m[3]}


def summary_problems(now, before, renamed=None):
    """`renamed`: {new path: old path} for lines whose title stayed and whose path changed."""
    renamed = renamed or {}
    out = []
    for path in now:
        if path in before:
            continue
        why = (f" It was the line for {renamed[path]}, renamed; that line was already one the book did not need."
               if path in renamed else "")
        out.append(f"{SUMMARY}: the new line for {path} lists a page the book adds on its own (a page in concepts/ "
                   f"or how-to/, or in a folder beside its parent page). Remove the line; the page joins its part by "
                   f"its title.{why}")
    return out


def carries_other_prs(branch, labels):
    """Whether this pull request is a rollup or an upstream sync, by its label or its branch name."""
    return bool(CARRIER_LABELS & set(labels)) or bool(branch and CARRIER_BRANCH.fullmatch(branch))


def lang_problems(changed, carrier):
    """`changed`: the paths under launcher/lang/ this change touches; `carrier`: a rollup or a sync."""
    if len(changed) <= LANG_LIMIT or carrier:
        return []
    return [f"{LANG}: this change touches {len(changed)} language files and at most {LANG_LIMIT} may change outside "
            "a rollup or an upstream sync (a `rollup` or `sync` label). A label missing from a language shows its "
            "English `tr()` text, so add a new label only to the languages you translate, and leave the other files "
            "alone."]


def notes_problems(before, now):
    if before is None or now is None or now <= before:
        return []
    return [f"{NOTES}: {now} lines, {before} on the base; this file may not grow. Put prose about one hook in that "
            "hook's own `#[notes]`, and other prose in a `Notes` trait in a new file under "
            "crates/horizon-mod-vocab/src/hooks/."]


def line_count(text):
    return None if text is None else merge_base.lines(text)


def at(root, rev, path):
    """`path`'s text at `rev`, or None where it does not exist."""
    shown = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=root, capture_output=True, encoding="utf-8")
    return shown.stdout if shown.returncode == 0 else None


def problems(root, merge_base, carrier=False):
    src = root / BOOK_SRC
    now_summary, base_summary = (root / SUMMARY).read_text(), at(root, merge_base, SUMMARY) or ""
    now_paths, base_paths = listed_paths(now_summary), listed_paths(base_summary)
    renamed = {path: base_paths[title] for title, path in now_paths.items()
               if title in base_paths and base_paths[title] != path}
    out = summary_problems(redundant_summary_lines(now_summary, src),
                           redundant_summary_lines(base_summary, src) if base_summary else [], renamed)
    changed = git(root, "diff", "--name-only", "--no-renames", merge_base, "--", LANG).split()
    out += lang_problems(changed, carrier)
    notes = root / NOTES
    out += notes_problems(line_count(at(root, merge_base, NOTES)),
                          line_count(notes.read_text()) if notes.is_file() else None)
    return out


def main(argv=None, root=ROOT):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="the git ref the branch starts from (CI: the pull request's base)")
    parser.add_argument("--pull-request-merge", action="store_true", help=merge_base.PULL_REQUEST_MERGE_HELP)
    parser.add_argument("--branch", default="", help="the pull request's head branch, when it is in this repository")
    parser.add_argument("--labels", default="", help="the pull request's labels, separated by spaces")
    args = parser.parse_args(argv)
    if not args.base:
        print("Hotspot guards: no base to compare with, nothing checked.")
        return 0
    if git(root, "rev-parse", "--is-shallow-repository").strip() == "true":
        print("Hotspot guards failed: comparing with the base needs full history; fetch --unshallow first.")
        return 1
    base, error = merge_base.pull_request_base(args.base, args.pull_request_merge)
    if error:
        print(f"Hotspot guards failed: {error}")
        return 1
    found = problems(root, git(root, "merge-base", base, "HEAD").strip(),
                     carries_other_prs(args.branch, args.labels.split()))
    for problem in found:
        print(problem)
    print("Hotspot guards: " + (f"{len(found)} problem(s)" if found else "pass"))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
