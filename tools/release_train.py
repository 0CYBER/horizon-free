#!/usr/bin/env python3
"""Decides when Horizon releases, names the release and writes its notes (.github/workflows/release.yml).

    python3 tools/release_train.py decide --kind game [--head SHA] [--force]
    python3 tools/release_train.py decide --kind launcher
    python3 tools/release_train.py notes --kind game --tag v2026.10.15 --out dist/notes
    python3 tools/release_train.py notes --kind game --tag v2026.10.15 --head v2026.10.15 --stable   # at promotion

A kind releases when a pull request merged into main since its last published release changed one of its
player-facing paths (GAME_PATHS, LAUNCHER_PATHS) and the newest of them merged at least DEBOUNCE ago (a burst
of merges becomes one release), or the oldest at least MAX_WAIT ago (merges that never pause still ship), or when one
of those pull requests is labelled release-now. Pull requests labelled skip-release (work players don't see) don't count
towards a release and aren't listed in the notes. RELEASES_PAUSED=1 (a repository variable) stops automatic releases.

Game releases are tagged vYYYY.MM.DD, then .01, .02 the same UTC day; launcher releases launcher-vX.Y.Z, the
patch after the last launcher tag, or the next minor when a merged pull request is labelled feature. Notes:
"### New" lists the pull requests labelled feature, "### Fixed" the others, "### Known issues" the bullets of
docs/known-issues.md. Game releases go out as betas (GitHub pre-releases) and tools/release_channels.py promotes
them to stable; a beta's notes cover what changed since the release before it (beta or stable), a promoted
release's (--stable) everything since the last stable one. See docs/release-flow.md.

Prints the decision as JSON and writes it to $GITHUB_OUTPUT when set. Needs git history and the gh CLI.
"""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REPO = os.environ.get("GITHUB_REPOSITORY", "ludovic111/horizon")
DEBOUNCE = dt.timedelta(minutes=15)
MAX_WAIT = dt.timedelta(hours=1)
# Kept as typed in release titles, where the clauses after the first are lowercased.
PROPER = {"Horizon", "Horizon's", "Minecraft", "Mojang", "Microsoft", "Java", "Xbox", "PlayStation", "Steam", "Windows",
          "Linux", "LAN", "Kira", "Discord", "Wayland", "Hyprland", "F3", "GUI", "UI"}
EXPORTERS = "mods/horizon/src/test/java/dev/horizonlab/parity/"

# What players run. Documentation (*.md), tests, benchmarks and harness probes (examples/) never count.
GAME_PATHS = ["crates/", "xtask/", "packaging/", "Cargo.toml", "Cargo.lock", "research-lock.json"]
# Every crate the launcher builds in (cargo metadata --manifest-path launcher/Cargo.toml: the path packages).
LAUNCHER_PATHS = [
    "launcher/",
    "crates/horizon-bench/",
    "crates/horizon-friends/",
    "crates/horizon-mods/",
    "crates/horizon-port-pack/",
    "crates/horizon-prepare/",
    "crates/horizon-release-sign/",
    "crates/horizon-telemetry/",
    "crates/horizon-theme/",
]
LAUNCHER_IGNORED = ["launcher/links.json", "launcher/docs/", "launcher/tests/"]

FOOTER = {
    "game": "Install or update it from the Horizon launcher: it installs new versions by itself. "
            "New here? Get the launcher at https://playhorizon.dev. You need your own copy of Minecraft: Java Edition.",
    "beta": "This is a beta build: launchers with Beta builds turned on in Settings install it now; everyone gets it "
            "once it has been out 6 hours without a release-blocker.",
    "launcher": "Your launcher updates itself. New here? Download the archive for your system below, extract it, "
                "and run `horizon-launcher`. Website: https://playhorizon.dev",
}
DISCLAIMER = "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft."


def run(*command, check=True):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if check and result.returncode:
        raise SystemExit(f"{' '.join(command)}: {result.stderr.strip()}")
    return result.stdout


def gh_json(*arguments):
    return json.loads(run("gh", *arguments) or "null")


def exporters():
    """The exporter and oracle sources horizon-prepare compiles: those its sources name."""
    names = set()
    for source in (ROOT / "crates" / "horizon-prepare" / "src").glob("*.rs"):
        names.update(re.findall(r'"([A-Z]\w+\.java)"', source.read_text()))
    return {EXPORTERS + name for name in names}


def player_facing(path, kind, exporter_paths=frozenset()):
    """Whether a change to path ships to players in a release of this kind."""
    if path.endswith(".md") or re.match(r"(crates|launcher)/([\w-]+/)?(examples|tests|benches)/", path):
        return False
    if kind == "launcher":
        return any(path.startswith(p) for p in LAUNCHER_PATHS) and not any(path.startswith(p) for p in LAUNCHER_IGNORED)
    return any(path == p or path.startswith(p) for p in GAME_PATHS) or path in exporter_paths


def is_game_tag(tag):
    return re.fullmatch(r"v\d{4}\.\d{2}\.\d{2}(\.\d+)?", tag) is not None


def is_launcher_tag(tag):
    return re.fullmatch(r"launcher-v\d+\.\d+\.\d+", tag) is not None


def tag_key(tag):
    return tuple(int(part) for part in re.findall(r"\d+", tag))


def next_game_tag(tags, today):
    """vYYYY.MM.DD, or .01, .02... when that day already has a tag (drafts and rolled-back releases included).

    The counter has two digits because GitHub's releases page sorts a day's tags as text: .9 went above .18.
    Every reader compares the numbers as integers, so the unpadded tags from before still order correctly."""
    day = f"v{today:%Y.%m.%d}"
    taken = [tag for tag in tags if tag == day or tag.startswith(day + ".")]
    if not taken:
        return day
    return f"{day}.{max(int(tag.split('.')[3]) if tag.count('.') == 3 else 0 for tag in taken) + 1:02d}"


def next_launcher_tag(tags, minor):
    versions = sorted((tag_key(tag) for tag in tags if is_launcher_tag(tag)), reverse=True)
    major, minor_, patch = versions[0] if versions else (0, 1, 0)
    return f"launcher-v{major}.{minor_ + 1}.0" if minor else f"launcher-v{major}.{minor_}.{patch + 1}"


def merged_pull_requests(base, head):
    """(number, merge commit) of the pull requests merged into main between base and head, oldest first."""
    found = []
    for line in run("git", "log", "--first-parent", "--reverse", "--format=%H%x09%s", f"{base}..{head}").splitlines():
        commit, _, subject = line.partition("\t")
        match = re.match(r"Merge pull request #(\d+) ", subject) or re.search(r"\(#(\d+)\)$", subject)
        if match:
            found.append((int(match.group(1)), commit))
    return found


def changed_paths(commit):
    """The files a pull request changed: its merge commit against main before it (first parent)."""
    return run("git", "diff", "--name-only", f"{commit}^1", commit).split()


def pull_request(number):
    data = gh_json("pr", "view", str(number), "--repo", REPO, "--json",
                   "number,title,url,labels,closingIssuesReferences,author,mergedAt")
    return {
        "number": data["number"],
        "title": data["title"].strip(),
        "url": data["url"],
        "labels": sorted(label["name"] for label in data["labels"]),
        "issues": sorted(issue["number"] for issue in data.get("closingIssuesReferences") or []),
        "author": (data.get("author") or {}).get("login", ""),
        "merged_at": dt.datetime.fromisoformat(data["mergedAt"].replace("Z", "+00:00")) if data.get("mergedAt") else None,
    }


def published_releases(betas=True):
    """Published (not draft) releases with their tags and dates; without betas (pre-releases) when betas is False."""
    releases = gh_json("api", f"repos/{REPO}/releases?per_page=100", "--paginate", "--slurp")
    flat = [release for page in releases for release in page]
    return [release for release in flat if not release["draft"] and (betas or not release["prerelease"])]


def last_release(releases, kind):
    keep = is_game_tag if kind == "game" else is_launcher_tag
    ours = [release for release in releases if keep(release["tag_name"])]
    return max(ours, key=lambda release: tag_key(release["tag_name"]), default=None)


def decision(kind, prs, now, paused=False, force=False):
    """Whether to release now, and why. prs: the merged pull requests since the last release, each with the
    files it changed under "paths" (player-facing ones only) and when it merged ("merged_at")."""
    counted = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    urgent = [pr for pr in counted if "release-now" in pr["labels"]]
    merged = [pr["merged_at"] for pr in counted if pr.get("merged_at")]
    if force:
        return True, "started by hand with force"
    if paused:
        return False, "RELEASES_PAUSED is set"
    if not counted:
        return False, "no merged pull request changed what players run"
    if urgent:
        return True, "release-now: " + ", ".join(f"#{pr['number']}" for pr in urgent)
    if merged and now - max(merged) < DEBOUNCE and now - min(merged) < MAX_WAIT:
        wait = DEBOUNCE - (now - max(merged))
        return False, f"{len(counted)} pull requests: waiting {wait.seconds // 60 + 1} more minutes for merges to settle"
    return True, f"{len(counted)} pull requests changed what players run"


def clause(title):
    """The start of a pull request title, for a release's name: "Nether portals: vanilla timing" -> "Nether portals"."""
    title = re.sub(r"^(?:fix|fixes|fixed)\s+(?:the\s+)?", "", title.strip(), flags=re.I)
    return re.split(r"[:;(]| — | - ", title, maxsplit=1)[0].strip().rstrip(".")


def summary(new, fixed, limit=72):
    """A short name for the release, from its pull request titles, features first."""
    titles = []
    for pr in new[::-1] + fixed[::-1]:  # several pull requests of one feature ("Controllers: ...") name it once
        title = clause(pr["title"])
        if title.lower() not in (t.lower() for t in titles):
            titles.append(title)
    shown, text = [], ""
    for title in titles:
        first = title.split(" ", 1)[0]
        if shown and title[1:2].islower() and first not in PROPER:
            title = title[0].lower() + title[1:]
        candidate = ", ".join(shown + [title])
        if shown and len(candidate) > limit - 20:
            break
        shown.append(title)
        text = candidate
    rest = len(titles) - len(shown)
    if rest:
        noun = ("change" if new else "fix") + ("" if rest == 1 else "es" if not new else "s")
        text += f" and {rest} more {noun}"
    return text


def bullet(pr):
    """A pull request as a line of notes: its title (written for people), its number and the issues it closes."""
    title = re.sub(r"\s*\((?:fixes|closes) #\d+\)|,? stacked on #\d+", "", pr["title"]).strip().rstrip(".")
    issues = "".join(f", fixes #{issue}" for issue in pr["issues"])
    return f"- {title} (#{pr['number']}{issues})"


def known_issues(path=ROOT / "docs" / "known-issues.md"):
    if not path.exists():
        return []
    return [line.rstrip() for line in path.read_text().splitlines() if line.startswith("- ")]


def notes(kind, version, prs, issues, beta=False):
    """(title, body) of a release; a game beta's say it is one."""
    shown = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    if kind == "game":
        title = f"Horizon {version.removeprefix('v')}" + (f": {summary(new, fixed)}" if shown else "")
    else:
        title = f"Horizon launcher {version.removeprefix('launcher-v')}"
    parts = [] if shown else ["Rebuilt from the latest main."]
    if new:
        parts.append("### New\n" + "\n".join(map(bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(bullet, fixed)))
    if issues and kind == "game":
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != REPO.split("/")[0]})
    if contributors:
        parts.append("Thanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    if beta and kind == "game":
        parts.append(FOOTER["beta"])
    parts.append(FOOTER[kind])
    parts.append(DISCLAIMER)
    return title, "\n\n".join(parts) + "\n"


def gather(kind, head, releases=None, betas=True):
    """The last published release of this kind (stable only when betas is False) and the pull requests merged
    since, with their player-facing paths."""
    releases = published_releases(betas) if releases is None else releases
    last = last_release(releases, kind)
    base = last["tag_name"] if last else run("git", "rev-list", "--max-parents=0", head).split()[0]
    exporter_paths = exporters()
    prs = []
    for number, commit in merged_pull_requests(base, head):
        pr = pull_request(number)
        pr["paths"] = [path for path in changed_paths(commit) if player_facing(path, kind, exporter_paths)]
        prs.append(pr)
    return last, prs


def green_main_head():
    """The newest commit on main whose checks passed: what an automatic release builds."""
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", "checks", "--branch", "main", "--event", "push",
                   "--limit", "10", "--json", "headSha,status,conclusion")
    for entry in runs:
        if entry["status"] == "completed":
            return entry["headSha"] if entry["conclusion"] == "success" else None
    return None


def write_outputs(values):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            for name, value in values.items():
                out.write(f"{name}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["decide", "notes"])
    parser.add_argument("--kind", choices=["game", "launcher"], required=True)
    parser.add_argument("--head", help="the commit to release (default: the newest commit on main whose checks passed)")
    parser.add_argument("--tag", help="notes: the release's tag")
    parser.add_argument("--force", action="store_true", help="release even without player-facing changes, before merges settle or while paused")
    parser.add_argument("--out", type=Path, help="notes: write title.txt and body.md here")
    parser.add_argument("--now", help="the time to decide at (ISO 8601; default now)")
    parser.add_argument("--stable", action="store_true",
                        help="notes: for promoting a beta to stable, so since the last stable release, without the beta line")
    args = parser.parse_args()
    now = dt.datetime.fromisoformat(args.now) if args.now else dt.datetime.now(dt.timezone.utc)

    head = args.head or green_main_head()
    if not head:
        result = {"release": False, "reason": "main's latest checks did not pass"}
        print(json.dumps(result, indent=1))
        write_outputs({"release": "false", "reason": result["reason"]})
        return 0
    head = run("git", "rev-parse", head).strip()
    last, prs = gather(args.kind, head, betas=not args.stable)
    if args.command == "notes":
        version = args.tag or "unreleased"
        title, body = notes(args.kind, version, prs, known_issues(), beta=not args.stable)
        if args.out:
            args.out.mkdir(parents=True, exist_ok=True)
            (args.out / "title.txt").write_text(title + "\n")
            (args.out / "body.md").write_text(body)
        print(title + "\n\n" + body)
        return 0

    release, reason = decision(args.kind, prs, now, os.environ.get("RELEASES_PAUSED") == "1", args.force)
    tags = run("git", "tag", "--list").split()
    minor = any("feature" in pr["labels"] for pr in prs if pr["paths"])
    tag = next_game_tag(tags, now.date()) if args.kind == "game" else next_launcher_tag(tags, minor)
    result = {
        "release": release, "reason": reason, "tag": tag, "head": head,
        "last": last["tag_name"] if last else None, "last_published": last["published_at"] if last else None,
        "pull_requests": [{key: pr[key] for key in ("number", "title", "labels", "paths")} for pr in prs],
    }
    print(json.dumps(result, indent=1))
    write_outputs({"release": str(release).lower(), "reason": reason, "tag": tag, "head": head})
    return 0


if __name__ == "__main__":
    sys.exit(main())
