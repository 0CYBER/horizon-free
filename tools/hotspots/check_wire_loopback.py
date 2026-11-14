#!/usr/bin/env python3
"""The wire_loopback test root gains no module line and no test.

crates/horizon-mod-wasm/wire-loopback/build.rs declares one module per file of its tests/wire_loopback/, so
the root may not name one of them with a hand-written `#[path = "wire_loopback/..."]` line, and its number of
`#[test]` attributes may not exceed the number at the branch's merge base. Independent pull requests that each
appended to the root kept conflicting; a file of their own does not. The target's old place in horizon-mod-wasm's
tests/ may not come back: nothing compiles a file there, so a test carried to it would vanish silently. The guest's one
`Hooks` object is assembled by build.rs from the folder's files: a feature writes its hook methods in
`impl <Feature>Hooks { pub(super) fn <hook>(&self, ..) }` (inherent, on a unit struct of its own file) and never hand-writes `impl Hooks for Mod`
or a second `__horizon_mod_hooks`. `register` is the one method several files may define; they run
in file-name order. Without
--base the test count is not checked (main's pushes have nothing to compare with).

    python3 tools/hotspots/check_wire_loopback.py --base origin/main
"""

import argparse
import importlib.util
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
# The merge-base rules (--pull-request-merge) are check_file_size.py's, shared by every --base gate.
_spec = importlib.util.spec_from_file_location("check_file_size", ROOT / "tools" / "check_file_size.py")
check_file_size = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_file_size)

ROOT_FILE = "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback.rs"
FOLDER = "crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback/"
ADVICE = f"add {FOLDER}<topic>.rs; it is discovered"
RETIRED = ("crates/horizon-mod-wasm/tests/wire_loopback.rs", "crates/horizon-mod-wasm/tests/wire_loopback")
MODULE_LINE = re.compile(r'#\[path\s*=\s*"wire_loopback/')
HOOKS_ADVICE = (f"put the hook methods in `impl <Feature>Hooks {{ pub(super) fn <hook>(&self, ..) }}` (a unit struct of that file) in "
                f"{FOLDER}<topic>.rs; build.rs assembles the one Hooks object (`register` is the one method several "
                "files may define; they run in file-name order)")
IMPL_HOOKS = re.compile(r"^\s*impl(?:<[^>]*>)?\s+(?:[\w:]*::)?Hooks\s+for\s+(\w+)", re.M)
HAND_EXPORT = re.compile(r"__horizon_mod_hooks")
TEST = re.compile(r"^\s*#\[test\]", re.M)


def problems(now, before):
    """What `now`, the root's text, does wrong; `before` is its text at the merge base, or None for no comparison."""
    found = [f"{ROOT_FILE}:{number}: hand-written module line; {ADVICE}"
             for number, line in enumerate(now.splitlines(), 1) if MODULE_LINE.search(line)]
    if before is not None:
        tests, base_tests = len(TEST.findall(now)), len(TEST.findall(before))
        if tests > base_tests:
            found.append(f"{ROOT_FILE}: {tests} #[test]s, {base_tests} at the merge base; {ADVICE}")
    return found


def hook_problems(name, text, is_root):
    """Hand-written Hooks objects in `text`, the root's or a folder file's, `name` being its path."""
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        match = IMPL_HOOKS.match(line)
        if match and (match.group(1) == "Mod" or match.group(1).endswith("Hooks")):
            found.append(f"{name}:{number}: hand-written `impl Hooks for {match.group(1)}`; {HOOKS_ADVICE}")
        if HAND_EXPORT.search(line) and not (is_root and "fn __horizon_mod_hooks() -> Box<dyn Hooks>" in line):
            found.append(f"{name}:{number}: hand-written hooks export; {HOOKS_ADVICE}")
    return found


def folder_hook_problems(root):
    """The Hooks rules over every file of the test folder under `root`, the repository's top folder."""
    folder = root / FOLDER
    return [problem for file in sorted(folder.glob("*.rs"))
            for problem in hook_problems(file.relative_to(root).as_posix(), file.read_text(encoding="utf-8"), False)]


def retired(root):
    """The files at the target's old place under `root`, the repository's top folder: nothing compiles them."""
    found = []
    for old in RETIRED:
        path = root / old
        files = sorted(path.rglob("*")) if path.is_dir() else [path] if path.exists() else []
        found += [f"{file.relative_to(root).as_posix()}: nothing compiles this; move it to {FOLDER}<topic>.rs"
                  for file in files if file.is_file()]
    return found


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="the git ref the branch starts from (CI: the pull request's base)")
    parser.add_argument("--pull-request-merge", action="store_true", help=check_file_size.PULL_REQUEST_MERGE_HELP)
    args = parser.parse_args(argv)
    before = None
    if args.base:
        base, error = check_file_size.pull_request_base(args.base, args.pull_request_merge)
        if error:
            print(f"wire_loopback check failed: {error}")
            return 1
        merge_base = check_file_size.git("merge-base", base, "HEAD").strip()
        # The root's path at the merge base: its own, or the one it was renamed from since.
        renames = check_file_size.git("diff", "--name-status", "-M", "--diff-filter=R", merge_base, "HEAD")
        old = next((line.split("\t")[1] for line in renames.splitlines() if line.split("\t")[-1] == ROOT_FILE),
                   ROOT_FILE)
        shown = subprocess.run(["git", "show", f"{merge_base}:{old}"], capture_output=True, encoding="utf-8", cwd=ROOT)
        if shown.returncode != 0:
            print(f"wire_loopback check failed: cannot read {old} at the merge base {merge_base} "
                  f"(a shallow clone? fetch --unshallow): {shown.stderr.strip()}")
            return 1
        before = shown.stdout
    root_text = (ROOT / ROOT_FILE).read_text(encoding="utf-8")
    found = (retired(ROOT) + problems(root_text, before) + hook_problems(ROOT_FILE, root_text, True)
             + folder_hook_problems(ROOT))
    for problem in found:
        print(problem)
    if found:
        return 1
    print("wire_loopback root: no module line, no hand-written Hooks" + ("" if before is None else ", no new test") + "; old place empty.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
