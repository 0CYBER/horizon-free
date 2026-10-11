#!/usr/bin/env python3
"""Manage parallel parity work: one worktree and branch per area, one integration branch.

    python3 tools/supervise.py status             # every worktree: branch, progress, last gate
    python3 tools/supervise.py new dispensers     # worktree ../dispensers on parity/dispensers, from the integration branch
    python3 tools/supervise.py save [AREA ...]    # commit and push unfinished work (before stopping agents)
    python3 tools/supervise.py integrate parity/dispensers   # merge into the integration branch if the gate passes

The integration branch (`parity-base` by default) is checked out in its own
worktree; `integrate` merges there, runs tools/gate.py and pushes the
integration branch only when the gate passes, allowing only the harnesses
that already fail on the integration branch to keep failing. Otherwise it undoes the merge
and leaves the branch for its owner to fix. Worktrees live next to the
integration worktree and share its vanilla data: their `.local/` entries
link to `shared-local/` in the same directory.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def git(*args, cwd=ROOT, check=True):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and result.returncode:
        raise SystemExit(f"git {' '.join(args)} failed in {cwd}:\n{result.stderr.strip()}")
    return result.stdout.strip()


def worktrees():
    found, current = [], {}
    for line in git("worktree", "list", "--porcelain").splitlines() + [""]:
        if not line:
            if current:
                found.append(current)
            current = {}
        elif line.startswith("worktree "):
            current["path"] = Path(line[len("worktree "):])
        elif line.startswith("branch "):
            current["branch"] = line[len("branch refs/heads/"):]
    return found


def integration(base):
    for tree in worktrees():
        if tree.get("branch") == base:
            return tree["path"]
    raise SystemExit(f"No worktree has {base} checked out; create one with: git worktree add <path> {base}")


def count(range_, cwd):
    return int(git("rev-list", "--count", range_, cwd=cwd, check=False) or 0)


def status(args):
    base_tree = integration(args.base)
    print(f"{'worktree':<16} {'branch':<26} {'ahead':>5} {'behind':>6} {'dirty':>5} {'unpushed':>8}  gate")
    for tree in worktrees():
        path, branch = tree["path"], tree.get("branch", "(detached)")
        if path.parent != base_tree.parent and path != base_tree:
            continue
        dirty = len(git("status", "--porcelain", cwd=path).splitlines())
        remote = f"origin/{args.push_to if branch == args.base else branch}"
        unpushed = count(f"{remote}..HEAD", path) if git("rev-parse", "--verify", "--quiet", remote, cwd=path, check=False) else "new"
        gate = path / ".local" / "gate.json"
        verdict = "never run"
        if gate.exists():
            result = json.loads(gate.read_text())
            verdict = ("passed" if result["passed"] else "failed") + ("" if result["parity"] else " (no harnesses)")
            if result["commit"] != git("rev-parse", "HEAD", cwd=path) or result["dirty"]:
                verdict += ", older commit"
        print(f"{path.name:<16} {branch:<26} {count(f'{args.base}..HEAD', path):>5} {count(f'HEAD..{args.base}', path):>6}"
              f" {dirty:>5} {unpushed:>8}  {verdict}")


def new(args):
    base_tree = integration(args.base)
    path = base_tree.parent / args.area
    branch = f"parity/{args.area}"
    if path.exists():
        raise SystemExit(f"{path} already exists")
    git("worktree", "add", "-b", branch, str(path), args.base)
    shared = base_tree.parent / "shared-local"
    (path / ".local").mkdir()
    for entry in sorted(shared.iterdir()) if shared.is_dir() else []:
        (path / ".local" / entry.name).symlink_to(entry)
    print(f"{path} on {branch}, from {args.base} at {git('rev-parse', '--short', 'HEAD', cwd=path)}")


def save(args):
    base_tree = integration(args.base)
    for tree in worktrees():
        path, branch = tree["path"], tree.get("branch")
        if not branch or not branch.startswith("parity/") or path.parent != base_tree.parent:
            continue
        if args.areas and path.name not in args.areas:
            continue
        if git("status", "--porcelain", cwd=path):
            git("add", "-A", cwd=path)
            git("commit", "-q", "-m", f"WIP: unfinished {path.name} parity work\n\nWork in progress saved when its agent "
                "stopped; may not build or pass its harnesses yet.\n\n"
                "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>", cwd=path)
            print(f"{path.name}: committed unfinished work")
        git("push", "-q", "origin", f"HEAD:{branch}", cwd=path)
        print(f"{path.name}: {branch} pushed at {git('rev-parse', '--short', 'HEAD', cwd=path)}")


def integrate(args):
    base_tree = integration(args.base)
    if git("status", "--porcelain", "--untracked-files=no", cwd=base_tree):
        raise SystemExit(f"{base_tree} has uncommitted changes; commit or stash them first")
    before = git("rev-parse", "HEAD", cwd=base_tree)
    state = base_tree / ".local/parity-runner/state.json"
    known = sorted(name for name, record in json.loads(state.read_text())["harnesses"].items()
                   if record["status"] != "pass") if state.exists() else []
    merge = subprocess.run(["git", "merge", "--no-ff", "--no-edit", args.branch], cwd=base_tree, capture_output=True, text=True)
    if merge.returncode:
        conflicts = git("diff", "--name-only", "--diff-filter=U", cwd=base_tree, check=False)
        git("merge", "--abort", cwd=base_tree, check=False)
        print(f"{args.branch} does not merge cleanly into {args.base}. Conflicts:\n{conflicts or merge.stdout + merge.stderr}")
        print(f"Merge {args.base} into {args.branch}, resolve them there, and integrate again.")
        return 2
    if git("rev-parse", "HEAD", cwd=base_tree) == before:
        print(f"{args.branch} has nothing new for {args.base}.")
        return 0
    gate = [sys.executable, "tools/gate.py", "-j", str(args.jobs), "--known-failures", ",".join(known)]
    gate += ["--no-parity"] if args.no_parity else []
    if subprocess.run(gate, cwd=base_tree).returncode:
        git("reset", "--hard", before, cwd=base_tree)
        print(f"The gate failed, so {args.branch} was not integrated; {args.base} is back at {before[:9]}.")
        return 1
    git("push", "-q", "origin", f"{args.base}:{args.push_to}", cwd=base_tree)
    print(f"Integrated {args.branch}; {args.base} pushed to origin/{args.push_to} at {git('rev-parse', '--short', 'HEAD', cwd=base_tree)}.")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", default="parity-base", help="the integration branch")
    parser.add_argument("--push-to", default="t3/achieve-full-parity", help="remote branch the integration branch is pushed to")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("status").set_defaults(run=status)
    command = commands.add_parser("new")
    command.add_argument("area")
    command.set_defaults(run=new)
    command = commands.add_parser("save")
    command.add_argument("areas", nargs="*")
    command.set_defaults(run=save)
    command = commands.add_parser("integrate")
    command.add_argument("branch")
    command.add_argument("-j", "--jobs", type=int, default=8)
    command.add_argument("--no-parity", action="store_true", help="skip the vanilla harnesses in the gate")
    command.set_defaults(run=integrate)
    args = parser.parse_args()
    return args.run(args) or 0


if __name__ == "__main__":
    sys.exit(main())
