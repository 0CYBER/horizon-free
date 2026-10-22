#!/usr/bin/env python3
"""Promotes Horizon's beta game releases to stable (.github/workflows/promote.yml, docs/release-flow.md).

    python3 tools/release_channels.py promote                   # the newest beta that is due, if any
    python3 tools/release_channels.py promote --tag v2026.10.19.03          # that beta now (by hand)
    python3 tools/release_channels.py promote --tag v2026.10.19.03 --force  # even if blocked or untested
    python3 tools/release_channels.py promote --dry-run         # say what would happen, change nothing

Every game release goes out as a beta (a GitHub pre-release). A beta is due for stable when:
- it is newer than the stable release (GitHub's "latest", which launchers and servers install by default),
- it has been out PROMOTE_AFTER (6 hours) without trouble; a promotion by hand (--tag) skips the wait,
- its release smoke test passed: release.yml attaches smoke-test.json only after the Mac mini ran the game,
- and no issue labelled release-blocker covers it. An open blocker holds every beta; one closed as completed keeps
  holding the betas published before it closed (they still have the bug); one closed as not planned holds nothing.
  Only people who can triage the repository can add the label, and the issue's text is never read: whoever wrote
  the issue could edit it afterwards, so it can't decide which betas go out.

The newest due beta is promoted: marked stable and latest, with notes covering everything since the previous
stable release (tools/release_train.py notes --stable). Older betas stay betas: the stable one contains them.
Nothing older than the current stable release is ever promoted, even with --force: GitHub's "latest" is what
horizon-server update and install.sh download, and it only moves forward.

When the oldest beta newer than the stable release has waited STALL_AFTER (24 hours), promotion has stalled
(blocked, smoke tests not running, the schedule off): an issue says so, and is closed once a beta is promoted.
"""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.release_train import is_game_tag, tag_key  # noqa: E402  (vYYYY.MM.DD[.N] and v0.2.0: docs/versioning.md)

REPO = os.environ.get("GITHUB_REPOSITORY", "ludovic111/horizon")
PROMOTE_AFTER = dt.timedelta(hours=6)
SMOKE_ASSET = "smoke-test.json"
BLOCKER = "release-blocker"
STALL_AFTER = dt.timedelta(hours=24)
STALLED = "Promotion to stable has stalled"
def parse_time(text):
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def covers(blocker, release):
    """Whether a release-blocker issue holds this beta back. Only its state and dates count, never its text."""
    if blocker.get("state") != "closed":
        return True
    closed = parse_time(blocker.get("closed_at"))
    return blocker.get("state_reason") == "completed" and bool(closed) and parse_time(release["published_at"]) < closed


def stable(releases):
    """The newest stable (non pre-release) game release."""
    stables = [r for r in releases if not r["prerelease"]]
    return max(stables, key=lambda r: tag_key(r["tag_name"]), default=None)


def reasons_against(release, current, blockers, now, by_hand=False):
    """Why this beta can't become stable now: an empty list when it can."""
    reasons = []
    if not release["prerelease"]:
        reasons.append("it is already stable")
    if current and tag_key(release["tag_name"]) <= tag_key(current["tag_name"]):
        reasons.append(f"it is not newer than the stable {current['tag_name']}")
    age = now - parse_time(release["published_at"])
    if not by_hand and age < PROMOTE_AFTER:
        left = PROMOTE_AFTER - age
        reasons.append(f"it has been out {age.seconds // 3600} h {age.seconds % 3600 // 60} min, "
                       f"{left.seconds // 3600} h {left.seconds % 3600 // 60} min to go")
    if not any(asset["name"] == SMOKE_ASSET for asset in release.get("assets", [])):
        reasons.append(f"it has no {SMOKE_ASSET}: the release smoke test did not run")
    for blocker in blockers:
        if covers(blocker, release):
            reasons.append(f"release-blocker #{blocker['number']}")
    return reasons


def choose(releases, blockers, now, tag=None, force=False):
    """(the beta to promote or None, a line saying why) for the game releases `releases`."""
    games = [r for r in releases if not r["draft"] and is_game_tag(r["tag_name"])]
    current = stable(games)
    if tag:
        release = next((r for r in games if r["tag_name"] == tag), None)
        if not release:
            return None, f"no published game release is tagged {tag}"
        reasons = reasons_against(release, current, blockers, now, by_hand=True)
        if force:
            # Even forced, "latest" only moves forward: horizon-server update and install.sh follow it.
            reasons = [r for r in reasons if r == "it is already stable" or r.startswith("it is not newer")]
        if reasons:
            return None, f"{tag} stays a beta: " + "; ".join(reasons)
        return release, f"{tag} promoted by hand" + (" with force" if force else "")
    betas = sorted((r for r in games if r["prerelease"]), key=lambda r: tag_key(r["tag_name"]), reverse=True)
    held = []
    for release in betas:
        reasons = reasons_against(release, current, blockers, now)
        if not reasons:
            return release, f"{release['tag_name']} has been out {PROMOTE_AFTER.seconds // 3600} h, passed its smoke test " \
                            "and no release-blocker covers it"
        held.append(f"{release['tag_name']}: " + "; ".join(reasons))
    return None, "no beta is due" + (" (" + " / ".join(held[:3]) + ")" if held else "")


def stalled(releases, now):
    """(the oldest beta newer than the stable release, how long it has waited) once that is STALL_AFTER, else None."""
    games = [r for r in releases if not r["draft"] and is_game_tag(r["tag_name"])]
    current = stable(games)
    waiting = [r for r in games if r["prerelease"] and (not current or tag_key(r["tag_name"]) > tag_key(current["tag_name"]))]
    if not waiting:
        return None
    oldest = min(waiting, key=lambda r: parse_time(r["published_at"]))
    age = now - parse_time(oldest["published_at"])
    return (oldest, age) if age >= STALL_AFTER else None


def stall_issue(releases, stall, reason):
    """The title and body of the issue saying promotion has stalled."""
    oldest, age = stall
    current = stable([r for r in releases if not r["draft"] and is_game_tag(r["tag_name"])])
    hours = int(age.total_seconds() // 3600)
    body = (f"Opened by .github/workflows/promote.yml, not by a person.\n\n"
            f"No beta has become stable for {hours} hours: {oldest['tag_name']} has waited since "
            f"{oldest['published_at']}, and launchers, servers and install.sh still get "
            f"{current['tag_name'] if current else 'no stable release'}.\n\n"
            f"The last check said: {reason}\n\n"
            "Usual causes: an open `release-blocker` issue (it holds every beta until it is closed), the Mac mini's "
            "release smoke test not running (no smoke-test.json on the betas), or the promote workflow disabled. "
            "To promote a beta by hand: `gh workflow run promote -f tag=<tag>` (add `-f force=true` to skip the "
            "smoke test and blockers). This issue closes itself once a beta is promoted.")
    return f"{STALLED}: {oldest['tag_name']} has waited {hours} h", body


def gh(*arguments, capture=True):
    result = subprocess.run(["gh", *arguments], cwd=ROOT, capture_output=capture, text=True)
    if result.returncode:
        raise SystemExit(f"gh {' '.join(arguments)}: {(result.stderr or '').strip()}")
    return result.stdout


def gh_pages(path):
    return [item for page in json.loads(gh("api", path, "--paginate", "--slurp") or "[]") for item in page]


def promote(release):
    """Marks the beta stable and latest, with notes covering everything since the previous stable release."""
    tag = release["tag_name"]
    with tempfile.TemporaryDirectory() as out:
        subprocess.run([sys.executable, str(ROOT / "tools" / "release_train.py"), "notes", "--kind", "game", "--tag", tag,
                        "--head", tag, "--stable", "--out", out], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        title = (Path(out) / "title.txt").read_text().strip()
        gh("release", "edit", tag, "--repo", REPO, "--prerelease=false", "--latest", "--title", title,
           "--notes-file", str(Path(out) / "body.md"))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["promote"])
    parser.add_argument("--tag", help="promote this beta now (by hand: no 6-hour wait)")
    parser.add_argument("--force", action="store_true", help="with --tag: even without a smoke test or with a blocker")
    parser.add_argument("--dry-run", action="store_true", help="say what would be promoted, change nothing")
    parser.add_argument("--now", help="the time to decide at (ISO 8601; default now)")
    args = parser.parse_args()
    if args.force and not args.tag:
        parser.error("--force needs --tag: only a named beta is forced")
    now = dt.datetime.fromisoformat(args.now) if args.now else dt.datetime.now(dt.timezone.utc)
    releases = gh_pages(f"repos/{REPO}/releases?per_page=100")
    blockers = [issue for issue in gh_pages(f"repos/{REPO}/issues?labels={BLOCKER}&state=all&per_page=100")
                if "pull_request" not in issue]
    release, reason = choose(releases, blockers, now, args.tag, args.force)
    if release and not args.dry_run:
        promote(release)
    line = ("Would promote: " if release and args.dry_run else "") + reason
    print(line)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(f"### {line}\n")
    report_stall(releases, release, reason, now, args.dry_run)
    return 1 if args.tag and not release else 0


def report_stall(releases, promoted, reason, now, dry_run):
    """Opens the stalled-promotion issue when one is due and none is open; closes it after a promotion."""
    open_issues = [issue for issue in gh_pages(f"repos/{REPO}/issues?state=open&per_page=100")
                   if "pull_request" not in issue and issue["title"].startswith(STALLED)]
    stall = None if promoted else stalled(releases, now)
    if stall and not open_issues:
        title, body = stall_issue(releases, stall, reason)
        print(("Would open: " if dry_run else "Opening: ") + title)
        if not dry_run:
            gh("issue", "create", "--repo", REPO, "--title", title, "--body", body)
    if promoted and not dry_run:
        for issue in open_issues:
            gh("issue", "close", str(issue["number"]), "--repo", REPO, "--comment",
               f"{promoted['tag_name']} is stable now (promote.yml).")


if __name__ == "__main__":
    sys.exit(main())
