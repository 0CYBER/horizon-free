#!/usr/bin/env python3
"""Upstream changes a sync merge did not carry into the fork.

    python3 tools/sync/dropped_upstream.py <sync merge commit>              # one row per file (TSV)
    python3 tools/sync/dropped_upstream.py <sync merge commit> --markdown   # grouped per upstream commit

The merge's first parent O is the fork, its second parent T is upstream; B = merge-base(O, T) is the previous sync
point. For every path P upstream changed in B..T, the fork's path P' is P, else where the fork moved it (git's
rename detection on B..O and B..M), else the merge's file with the same tail after `game/` or the same name holding
most of upstream's added lines, else the same crate's file holding a third of upstream's longer added lines (a file
the fork split or merged). Upstream's added lines are those in T:P and not in B:P (blank lines ignored); a change is
carried when they are all in M:P'. A file found only by the last two guesses is always listed, carried or not: a
wrong guess must not hide a lost change. A row is one of:
  dropped     M:P' is O:P' (the merge kept the fork's file) and upstream's added lines are missing
  partial     the merge changed P' but some upstream-added lines are missing
  guessed     P' was guessed (see above): check by hand that the change is there
  gone        the merge has no P' at all
  deleted-up  upstream deleted P and the merge still has it (or where the fork moved it)
  binary      upstream changed a binary P and M:P' differs from T:P

The comparison is per file and by line, so it over-reports (code the fork rewrote, moved or re-wrapped counts as
lost) and can under-report: a change whose added lines exist elsewhere in the file, a change that only removes or
reorders lines, and B is the previous sync point only while upstream never merged fork history. It lists candidates: each upstream commit it
names is then sorted by hand (`git show <commit>`, `git grep` at the merge) into carried, rightly kept fork side, or
lost, and the lost ones are re-applied. The sync PR body carries that "lost" table (handbook, section 7).
"""
import argparse
import subprocess
import sys


def git(*args: str, input: str | None = None) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True, errors="replace",
                          input=input).stdout


def blob(rev: str, path: str) -> str | None:
    out = subprocess.run(["git", "rev-parse", "-q", "--verify", f"{rev}:{path}"], capture_output=True, text=True)
    return out.stdout.strip() or None


def lines(rev: str, path: str | None) -> list[str]:
    b = blob(rev, path) if path else None
    return git("cat-file", "-p", b).splitlines() if b else []


def renames(a: str, b: str) -> dict[str, str]:
    out = git("-c", "diff.renameLimit=100000", "diff", "-M40%", "--name-status", "--diff-filter=R", a, b)
    return {old: new for _, old, new in (row.split("\t") for row in out.splitlines())}


def rows(merge: str) -> tuple[list[tuple[str, str, str, str, int, int, list[str]]], dict[str, str]]:
    """(kind, upstream path, fork path, whether the fork changed it since B, lost, added, upstream commits) per flagged
    file, and the commits B, O and T."""
    parents = git("rev-list", "--parents", "-n", "1", merge).split()[1:]
    if len(parents) != 2:
        sys.exit(f"{merge} is not a two-parent merge")
    o, t = parents
    b = git("merge-base", o, t).strip()
    moved = renames(b, o) | renames(b, merge)
    files = git("ls-tree", "-r", "--name-only", merge).splitlines()

    def relocated(path: str, added: list[str]) -> str | None:
        tail = ("/" + path).split("/game/", 1)[1] if "/game/" in "/" + path else None
        same_tail = [f for f in files if tail and f.endswith("/" + tail) and f != path]
        same_name = [f for f in files if f.rsplit("/", 1)[-1] == path.rsplit("/", 1)[-1] and f != path]
        scored = []
        for f in same_tail or same_name:
            have = set(lines(merge, f))
            scored.append((sum(line in have for line in added), f))
        best = max(scored, default=(0, None))
        if best[0] or same_tail:
            return best[1]
        longer = sorted({line.strip() for line in added if len(line.strip()) > 30})[:60]
        if not longer or not path.endswith(".rs"):
            return None
        crate = "/".join(path.split("/")[:2]) + "/" if path.count("/") >= 2 else ""
        out = subprocess.run(["git", "grep", "-c", "-F", "-f", "/dev/stdin", merge, "--", crate + "**.rs"],
                             input="\n".join(longer), capture_output=True, text=True).stdout
        hits = [(int(row.rsplit(":", 1)[1]), row.split(":", 1)[1].rsplit(":", 1)[0]) for row in out.splitlines()]
        best = max(hits, default=(0, None))
        return best[1] if best[0] * 3 >= len(longer) else None

    found = []
    for row in git("diff", "--name-status", "--no-renames", b, t).splitlines():
        status, path = row.split("\t")[0], row.split("\t")[-1]
        guessed = False
        if status == "D":
            fork_path = path if blob(merge, path) else moved.get(path)
            if not fork_path or not blob(merge, fork_path):
                continue
            kind, lost, added = "deleted-up", [], []
        elif "\0" in git("cat-file", "-p", f"{t}:{path}")[:8000]:
            fork_path = path if blob(merge, path) else moved.get(path)
            if fork_path and blob(merge, fork_path) == blob(t, path):
                continue
            kind, lost, added = "binary", [], []
        else:
            base = set(lines(b, path))
            added = [line for line in lines(t, path) if line.strip() and line not in base]
            if not added:
                continue
            fork_path = path if (blob(o, path) or blob(merge, path)) else moved.get(path)
            if not fork_path or not blob(merge, fork_path):
                fork_path, guessed = relocated(path, added), True
            if not fork_path or not blob(merge, fork_path):
                kind, lost = "gone", added
            else:
                merged = set(lines(merge, fork_path))
                lost = [line for line in added if line not in merged]
                if not lost and not guessed:
                    continue
                kind = "guessed" if guessed else "dropped" if blob(merge, fork_path) == blob(o, fork_path) else "partial"
        changed = "fork-changed" if blob(o, fork_path or path) != blob(b, path) else "fork-unchanged"
        # A change made only in an upstream merge's resolution has no non-merge commit: name the merge then.
        commits = (git("log", "--no-merges", "--format=%h", f"{b}..{t}", "--", path).split()
                   or git("log", "--format=%h", f"{b}..{t}", "--", path).split() or ["(no commit)"])
        found.append((kind, path, fork_path or "-", changed, len(lost), len(added), commits))
    return found, {"B": b, "O": o, "T": t}


def markdown(found, ends) -> str:
    by_commit: dict[str, list[str]] = {}
    for kind, path, fork_path, _, lost, added, commits in found:
        for commit in commits:
            where = path if fork_path in (path, "-") else f"{path} → {fork_path}"
            by_commit.setdefault(commit, []).append(f"{kind} {lost}/{added} {where}")
    out = [f"Upstream {ends['B'][:10]}..{ends['T'][:10]} into fork {ends['O'][:10]}: {len(found)} files, "
           f"{len(by_commit)} upstream commits to sort.", "",
           "| Upstream commit | Files (kind, lost/added lines) | Sorted as |", "| --- | --- | --- |"]
    def info(commit: str, field: str) -> str:
        return "" if commit == "(no commit)" else git("log", "-1", f"--format={field}", commit).strip()

    for commit, notes in sorted(by_commit.items(), key=lambda item: info(item[0], "%ct")):
        subject = info(commit, "%s").replace("|", "\\|")
        out.append(f"| `{commit}` {subject} | {'<br>'.join(notes)} | |")
    return "\n".join(out) + "\n"


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("merge", help="the sync's merge commit (first parent the fork, second upstream)")
    parser.add_argument("--markdown", action="store_true", help="one row per upstream commit, for the sync PR body")
    args = parser.parse_args(argv)
    found, ends = rows(args.merge)
    if args.markdown:
        print(markdown(found, ends), end="")
    else:
        for kind, path, fork_path, changed, lost, added, commits in found:
            print("\t".join([kind, path, fork_path, changed, f"{lost}/{added}", " ".join(commits)]))
    print(f"# {len(found)} files; B={ends['B'][:10]} O={ends['O'][:10]} T={ends['T'][:10]}", file=sys.stderr)


if __name__ == "__main__":
    main()
