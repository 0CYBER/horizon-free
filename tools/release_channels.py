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
- and no issue labelled release-blocker covers it. A blocker covers the betas from the version it names onwards
  (every beta when it names none), published while it is open; one closed as completed keeps covering the betas
  published before it closed (they still have the bug), one closed as not planned covers nothing.

The newest due beta is promoted: marked stable and latest, with notes covering everything since the previous
stable release (tools/release_train.py notes --stable). Older betas stay betas: the stable one contains them.
Issue text is only searched for version numbers; it never changes what this does beyond blocking.
"""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPO = os.environ.get("GITHUB_REPOSITORY", "ludovic111/horizon")
PROMOTE_AFTER = dt.timedelta(hours=6)
SMOKE_ASSET = "smoke-test.json"
BLOCKER = "release-blocker"
GAME_TAG = re.compile(r"v\d{4}\.\d{2}\.\d{2}(?:\.\d+)?")
# A version as people write it in an issue: with or without the v, a day's counter padded or not.
MENTION = re.compile(r"(?<![\w.])v?(\d{4})\.(\d{1,2})\.(\d{1,2})(?:\.(\d+))?(?!\w)")


def tag_key(tag):
    """A game tag's numbers, four of them (v2026.10.19 is (2026, 10, 19, 0)), so tags and mentions compare."""
    return padded(tuple(int(part) for part in re.findall(r"\d+", tag)))


def padded(version):
    return version + (0,) * (4 - len(version))


def parse_time(text):
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def mentioned_versions(text):
    """The game versions an issue names, as tag keys ((2026, 10, 19, 3) for v2026.10.19.03)."""
    return {padded(tuple(int(part) for part in match.groups() if part is not None)) for match in MENTION.finditer(text or "")}


def covers(blocker, release):
    """Whether a release-blocker issue holds this beta back."""
    if blocker.get("state") == "closed":
        closed = parse_time(blocker.get("closed_at"))
        if blocker.get("state_reason") != "completed" or not closed:
            return False
        if parse_time(release["published_at"]) >= closed:
            return False
    versions = mentioned_versions(f"{blocker.get('title', '')}\n{blocker.get('body') or ''}")
    if not versions:
        return True
    return any(tag_key(release["tag_name"]) >= version for version in versions)


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
    games = [r for r in releases if not r["draft"] and GAME_TAG.fullmatch(r["tag_name"])]
    current = stable(games)
    if tag:
        release = next((r for r in games if r["tag_name"] == tag), None)
        if not release:
            return None, f"no published game release is tagged {tag}"
        reasons = reasons_against(release, current, blockers, now, by_hand=True)
        if force:
            reasons = [r for r in reasons if r == "it is already stable"]
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
    return 1 if args.tag and not release else 0


if __name__ == "__main__":
    sys.exit(main())
