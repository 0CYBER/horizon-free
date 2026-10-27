#!/usr/bin/env python3
"""The opt-in Nightly channel (.github/workflows/nightly-build.yml, docs/release-flow.md "Nightly").

    nightly.py pick [--force]      which commit tonight's nightly builds: GitHub outputs and nightly-notes.md
    nightly.py rotate              renames the current `nightly` pre-release to nightly-<its date>, frees the tag
    nightly.py prune [--days 7]    deletes the nightly-<date> pre-releases and tags older than that, nothing else

A nightly builds the newest union branch on origin (`union/<agent>-<name>`: main plus pull requests the contribution
gate proved together), or main when no union is newer than main's head. It is published as the `nightly`
pre-release; the previous one stays a week as nightly-<date>. Runs from main's checkout with `gh`; it reads the
branch it picks with git and never runs anything from it.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass

DATED = re.compile(r"^nightly-(\d{4}-\d{2}-\d{2})$")
TITLE_DATE = re.compile(r"Nightly (\d{4}-\d{2}-\d{2})")


@dataclass
class Ref:
    name: str
    sha: str
    date: dt.datetime  # committer date


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def gh(*args: str, check: bool = True) -> str:
    out = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        sys.exit(f"gh {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout.strip() if out.returncode == 0 else ""


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b]).returncode == 0


def choose(main: Ref, unions: list[Ref], contains=is_ancestor) -> Ref:
    """The newest union whose head is newer than main's and holds something main lacks; else main."""
    for u in sorted(unions, key=lambda u: u.date, reverse=True):
        if u.date <= main.date:
            break
        if not contains(u.sha, main.sha):
            return u
    return main


def refs(pattern: str) -> list[Ref]:
    out = git("for-each-ref", "--format=%(refname:short) %(objectname) %(committerdate:iso-strict)", pattern)
    return [Ref(n.removeprefix("origin/"), s, dt.datetime.fromisoformat(d))
            for n, s, d in (line.split(" ", 2) for line in out.splitlines())]


def output(**values: str) -> None:
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as f:
        for k, v in values.items():
            f.write(f"{k}={v}\n")


def notes(today: str, chosen: Ref, held: list[dict]) -> str:
    lines = [f"Nightly {today} · {chosen.sha[:7]}: built from `{chosen.name}` ({chosen.sha}).", "",
             "**Opt-in, not a release.** Fixes the maintainers reviewed but haven't released yet: it may break. "
             "Turn it on in the launcher's settings; your release install and your worlds stay as they are. "
             "Report bugs with the version the game shows (\"Nightly <date> · <commit>\").", ""]
    if held:
        lines += [f"### Not on main yet ({len(held)})", ""]
        lines += [f"- {p['title']} (#{p['number']})" for p in held]
    else:
        lines += ["Everything in it is on main."]
    return "\n".join(lines) + "\n"


def pick(force: bool) -> None:
    git("fetch", "--quiet", "origin", "+refs/heads/main:refs/remotes/origin/main",
        "+refs/heads/union/*:refs/remotes/origin/union/*")
    main = refs("refs/remotes/origin/main")[0]
    chosen = choose(main, refs("refs/remotes/origin/union/"))
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    previous = git("ls-remote", "--tags", "origin", "refs/tags/nightly").split("\t")[0] or None
    build = force or previous != chosen.sha
    # The open pull requests this build holds beyond main: their heads are in it and not in main.
    prs = json.loads(gh("pr", "list", "--state", "open", "--limit", "400", "--json", "number,title,headRefOid"))
    held = []
    for pr in prs:
        oid = pr["headRefOid"]
        if subprocess.run(["git", "cat-file", "-e", oid], capture_output=True).returncode != 0:
            continue  # a fork's head that no fetched branch holds
        if is_ancestor(oid, chosen.sha) and not is_ancestor(oid, main.sha):
            held.append(pr)
    held.sort(key=lambda p: p["number"])
    with open("nightly-notes.md", "w") as f:
        f.write(notes(today, chosen, held))
    output(build=str(build).lower(), sha=chosen.sha, short=chosen.sha[:7], source=chosen.name, date=today,
           version=f"nightly-{today}", nightly=f"{today} {chosen.sha[:7]}")
    print(f"{'build' if build else 'skip (unchanged since the last nightly)'}: {chosen.name} {chosen.sha}, "
          f"{len(held)} open pull requests beyond main")


def rotate() -> None:
    """The current `nightly` becomes nightly-<its date>, so tonight's can take the tag."""
    current = gh("release", "view", "nightly", "--json", "name", check=False)
    if not current:
        return
    m = TITLE_DATE.search(json.loads(current)["name"])
    dated = f"nightly-{m.group(1)}" if m else f"nightly-{dt.date.today() - dt.timedelta(days=1)}"
    if gh("release", "view", dated, "--json", "tagName", check=False):
        gh("release", "delete", dated, "--cleanup-tag", "--yes")  # a second nightly the same day replaces the first
    gh("release", "edit", "nightly", "--tag", dated)
    gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/tags/nightly", check=False)
    print(f"nightly -> {dated}")


def stale(tags: list[dict], today: dt.date, days: int) -> list[str]:
    """The dated nightly pre-releases older than `days`: never a release, never `nightly` itself."""
    cutoff = today - dt.timedelta(days=days)
    return [r["tagName"] for r in tags if r["isPrerelease"] and (m := DATED.match(r["tagName"]))
            and dt.date.fromisoformat(m.group(1)) < cutoff]


def prune(days: int) -> None:
    tags = json.loads(gh("release", "list", "--limit", "200", "--json", "tagName,isPrerelease"))
    for tag in stale(tags, dt.datetime.now(dt.timezone.utc).date(), days):
        gh("release", "delete", tag, "--cleanup-tag", "--yes")
        print(f"deleted {tag}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick").add_argument("--force", action="store_true")
    sub.add_parser("rotate")
    sub.add_parser("prune").add_argument("--days", type=int, default=7)
    args = ap.parse_args()
    if args.cmd == "pick":
        pick(args.force)
    elif args.cmd == "rotate":
        rotate()
    else:
        prune(args.days)


if __name__ == "__main__":
    main()
