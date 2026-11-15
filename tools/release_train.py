#!/usr/bin/env python3
"""Decides when Horizon releases, names the release and writes its notes (.github/workflows/release.yml).

    python3 tools/release_train.py decide --kind game [--head SHA] [--force]
    python3 tools/release_train.py decide --kind game --channel nightly|snapshot [--head SHA] [--force] [--daily]
    python3 tools/release_train.py decide --kind launcher
    python3 tools/release_train.py notes --kind game --tag v2026.10.15 --out dist/notes
    python3 tools/release_train.py gate --kind game --smoke dist/smoke-test.json [--blockers]   # before publishing

The game ships only the Nightly since 2026-10-28 (Ludovic): when `decide --kind game` says so, release.yml starts
`nightly build` on the commit it decided (tools/ci/nightly.py), and "the last release" is the published Nightly.
On the fork (CI tiers, owner 2026-11-13) `--channel` names the kind of build: `nightly` takes `daily-green` and
refuses any other --head (an older commit had no daily run of its own, and no SDK); `snapshot` takes main's head (or
a --head on main) and refuses a commit whose gate jobs (dyna-checks' main-push lint, verify, verify-static, launcher)
did not all pass (pending is not passed); otherwise it always builds, as snapshot-<date>-<short> (no batch rules,
cap, pause or release-blocker: a snapshot is for testing main, never for players).

A kind releases when a pull request merged into main since its last published release changed one of its
player-facing paths (GAME_PATHS, LAUNCHER_PATHS), once BATCH of them are waiting and the newest merged at least
DEBOUNCE ago (a burst of merges becomes one release), or once the oldest has waited MAX_WAIT (a lone fix still ships),
or at once when one of those pull requests is labelled release-now. Pull requests labelled skip-release (work players don't see) don't count
towards a release and aren't listed in the notes. RELEASES_PAUSED=1 (a repository variable) stops automatic releases,
and an open issue labelled release-blocker stops game releases, even forced ones, until it is closed.

Tags (docs/versioning.md): with a release line in .github/release-line.txt (MAJOR.MINOR, e.g. 0.2), game releases
are vMAJOR.MINOR.PATCH and launcher releases launcher@MAJOR.MINOR.PATCH, each the patch after the last tag of its kind
on the line (MAJOR.MINOR.0 first: a line's first release goes out without waiting for a pull request). Without the
file, the tags from before 0.2: vYYYY.MM.DD, then .01, .02 the same UTC day, and launcher-vX.Y.Z, the patch after
the last launcher tag or the next minor when a merged pull request is labelled feature. Every 0.2 tag is newer than
every tag from before (tag_key). Notes:
"### New" lists the pull requests labelled feature, "### Fixed" the others, "### Known issues" the bullets of
docs/known-issues.md. Every release is published as the release, GitHub's latest, which launchers, servers and
install.sh take at once; one channel, no promotion. A game release is published only after `gate`: its smoke test
passed and no release-blocker is open. Notes cover everything since the last published release; the game
pre-releases of the old Alpha channel (v0.2.1 on) don't count, so the first release after them lists all of
it since v0.2.0. See docs/release-flow.md.

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
REPO = os.environ.get("GITHUB_REPOSITORY", "horizon-rust/horizon")
# The maintainer, left out of the notes' thanks (the repository's owner is the horizon-rust organization).
MAINTAINER = "ludovic111"
# Releases ship in batches, so players get a few releases a day rather than one per merge: BATCH player-facing pull
# requests, settled for DEBOUNCE, or whatever is waiting once the oldest has waited MAX_WAIT.
BATCH = 5
DEBOUNCE = dt.timedelta(minutes=5)
MAX_WAIT = dt.timedelta(hours=6)
# Kept as typed in release titles, where the clauses after the first are lowercased.
PROPER = {"Horizon", "Horizon's", "Minecraft", "Mojang", "Microsoft", "Java", "Xbox", "PlayStation", "Steam", "Windows",
          "Linux", "LAN", "Kira", "Discord", "Wayland", "Hyprland", "F3", "GUI", "UI"}
EXPORTERS = "mods/horizon/src/test/java/dev/horizonlab/parity/"
# The release line (docs/versioning.md): MAJOR.MINOR. Without the file, releases keep the tags from before 0.2.
LINE_FILE = ROOT / ".github" / "release-line.txt"
SEMVER = r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
# Semver's majors stay below this; a date tag's first number is a year.
FIRST_YEAR = 1000
GAME_PREFIX = "v"
LAUNCHER_PREFIX = "launcher@"
OLD_LAUNCHER_PREFIX = "launcher-v"

# What players run. Documentation (*.md), tests, benchmarks and harness probes (examples/) never count.
GAME_PATHS = ["crates/", "xtask/", "packaging/", "Cargo.toml", "Cargo.lock", "research-lock.json"]
# Every crate the launcher builds in (cargo metadata --manifest-path launcher/Cargo.toml: the path packages).
LAUNCHER_PATHS = [
    "launcher/",
    "crates/horizon-bench/",
    "crates/horizon-friends/",
    "crates/horizon-mcp/",
    "crates/horizon-mod-api-gen/",
    "crates/horizon-mod-package/",
    "crates/horizon-port-pack/",
    "crates/horizon-prepare/",
    "crates/horizon-release-sign/",
    "crates/horizon-system/",
    "crates/horizon-theme/",
    # Its icon, its menu entries and its AppImage are compiled or packaged into a release
    # (launcher/build.rs, src/desktop.rs, packaging/linux/appimage.sh).
    "packaging/icons/",
    "packaging/linux/",
]
LAUNCHER_IGNORED = ["launcher/links.json", "launcher/docs/", "launcher/tests/"]

# A game release is GitHub's latest (servers and install.sh follow it), so it is the first release people land on:
# its notes open with the way to the launcher (#1647). Raw HTML on one line, which GitHub shows and every launcher's
# notes view and one-line summary skip (pulldown-cmark's HTML events), so launchers never tell players to get
# themselves. The site's Releases page leaves it out too.
# The same warning closes the notes, above the disclaimer, for whoever scrolls past the changes to the files.
PLAYERS_FOOTER = ('<h1>Want to play? Download the <a href="https://playhorizon.dev">Horizon launcher</a>.</h1>'
                  '<h1>Do NOT run the files below. They will not work on their own.</h1>')
PLAYERS = (PLAYERS_FOOTER + '<p>The launcher installs this version and keeps it up to date. You need your own copy of '
           'Minecraft: Java Edition. The files below are what the launcher installs, and the dedicated server for '
           'people who host one.</p>')
LAUNCHER_FOOTER = ("Your launcher updates itself. New here? On Linux take the .AppImage below, make it executable "
                   "and run it; on Windows and macOS download the archive, extract it and run `horizon-launcher` "
                   "(Horizon.app on macOS). Website: https://playhorizon.dev")
DISCLAIMER = "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft."
# An open issue with this label holds every game release (docs/release-flow.md). Only people who can triage the
# repository can add it, and the issue's text is never read: its author could edit it afterwards.
BLOCKER = "release-blocker"


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


def semver(tag, prefix):
    """(major, minor, patch, pre-release or None) of a 0.2 tag `<prefix>X.Y.Z[-PRE]`, else None."""
    match = re.fullmatch(re.escape(prefix) + SEMVER, tag)
    if not match or int(match[1]) >= FIRST_YEAR:
        return None
    return int(match[1]), int(match[2]), int(match[3]), match[4]


def is_game_tag(tag):
    return re.fullmatch(r"v\d{4}\.\d{2}\.\d{2}(\.\d+)?", tag) is not None or semver(tag, GAME_PREFIX) is not None


def is_launcher_tag(tag):
    return re.fullmatch(r"launcher-v\d+\.\d+\.\d+", tag) is not None or semver(tag, LAUNCHER_PREFIX) is not None


def tag_key(tag):
    """A tag's place among releases (docs/versioning.md): the tags from before 0.2 (vYYYY.MM.DD[.N],
    launcher-vX.Y.Z) by their numbers, trailing zeros dropped (v2026.10.19 is v2026.10.19.0); then every 0.2 tag
    (vX.Y.Z, launcher@X.Y.Z) in semver's order, a pre-release (v0.3.0-rc.1) before its release."""
    version = semver(tag, GAME_PREFIX) or semver(tag, LAUNCHER_PREFIX)
    if version is None:
        numbers = [int(part) for part in re.findall(r"\d+", tag)]
        while numbers and numbers[-1] == 0:
            numbers.pop()
        return 0, tuple(numbers), ()
    *numbers, pre = version
    # A release follows its pre-releases; their identifiers compare one by one, numbers before words.
    rank = (1,) if pre is None else (0, tuple((0, int(i), "") if i.isdigit() else (1, 0, i) for i in pre.split(".")))
    return 1, tuple(numbers), rank


def release_line(path=LINE_FILE):
    """(major, minor) of the release line, or None without the file (the tags from before 0.2)."""
    if not path.exists():
        return None
    text = path.read_text().strip()
    match = re.fullmatch(r"(\d+)\.(\d+)", text)
    if not match:
        raise SystemExit(f"{path}: a release line is MAJOR.MINOR, like 0.2 (got {text!r})")
    return int(match[1]), int(match[2])


def next_on_line(tags, prefix, line):
    """<prefix>MAJOR.MINOR.PATCH: the patch after the newest release tagged on the line (drafts and rolled-back
    releases included; a hand-made pre-release such as v0.3.0-rc.1 is not one), or .0. Never a line below one that
    has tags: GitHub's latest release only moves forward."""
    major, minor = line
    versions = [v for v in (semver(tag, prefix) for tag in tags) if v]
    above = [v for v in versions if (v[0], v[1]) > line and v[3] is None]
    if above:
        raise SystemExit(f"{LINE_FILE.name} says {major}.{minor}, but {prefix}{'.'.join(map(str, above[0][:3]))} exists")
    patches = [v[2] for v in versions if (v[0], v[1]) == line and v[3] is None]
    return f"{prefix}{major}.{minor}.{max(patches) + 1 if patches else 0}"


def first_on_line(tags, prefix, line):
    """Whether no release of this kind is tagged on the line yet (its first goes out at once)."""
    return not any(v and (v[0], v[1]) == line and v[3] is None for v in (semver(tag, prefix) for tag in tags))


def next_game_tag(tags, today, line=None):
    """vMAJOR.MINOR.PATCH on a release line; before 0.2, vYYYY.MM.DD, or .01, .02... when that day already has a
    tag (drafts and rolled-back releases included).

    The counter has two digits because GitHub's releases page sorts a day's tags as text: .9 went above .18.
    Every reader compares the numbers as integers, so the unpadded tags from before still order correctly."""
    if line:
        return next_on_line(tags, GAME_PREFIX, line)
    day = f"v{today:%Y.%m.%d}"
    taken = [tag for tag in tags if tag == day or tag.startswith(day + ".")]
    if not taken:
        return day
    return f"{day}.{max(int(tag.split('.')[3]) if tag.count('.') == 3 else 0 for tag in taken) + 1:02d}"


def next_launcher_tag(tags, minor, line=None):
    """launcher@MAJOR.MINOR.PATCH on a release line (a feature doesn't change the line); before 0.2,
    launcher-vX.Y.Z, the next patch or with minor the next minor."""
    if line:
        return next_on_line(tags, LAUNCHER_PREFIX, line)
    versions = sorted((tuple(int(part) for part in re.findall(r"\d+", tag)) for tag in tags
                       if tag.startswith(OLD_LAUNCHER_PREFIX) and is_launcher_tag(tag)), reverse=True)
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


def merged_pull_request_metadata():
    """Repository-owned merge identities and note fields, without one network call per PR."""
    owner, name = REPO.split("/")
    query = """
    query($owner: String!, $name: String!, $endCursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequests(first: 100, states: MERGED, after: $endCursor) {
          nodes {
            number title url mergedAt mergeCommit { oid } author { login }
            labels(first: 100) { nodes { name } }
            closingIssuesReferences(first: 100) { nodes { number } }
          }
          pageInfo { hasNextPage endCursor }
        }
      }
    }
    """
    pages = gh_json("api", "graphql", "--paginate", "--slurp",
                    "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={query}")
    owned = {}
    for page in pages:
        for data in page["data"]["repository"]["pullRequests"]["nodes"]:
            # A historical merge commit can have been removed by a force-push. It cannot
            # match a reachable candidate; GraphQL explicitly permits a null commit.
            if data["mergeCommit"] is None:
                continue
            owned[(data["number"], data["mergeCommit"]["oid"])] = {
                "number": data["number"],
                "title": data["title"].strip(),
                "url": data["url"],
                "labels": sorted(label["name"] for label in data["labels"]["nodes"]),
                "issues": sorted(issue["number"] for issue in data["closingIssuesReferences"]["nodes"]),
                "author": (data["author"] or {}).get("login", ""),
                "merged_at": dt.datetime.fromisoformat(data["mergedAt"].replace("Z", "+00:00")),
            }
    return owned


def published_releases():
    """Published releases with their tags and dates: no drafts, and no pre-releases (the old Alpha channel's)."""
    releases = gh_json("api", f"repos/{REPO}/releases?per_page=100", "--paginate", "--slurp")
    flat = [release for page in releases for release in page]
    return [release for release in flat if not release["draft"] and not release["prerelease"]]


def open_blockers():
    """The numbers of the open issues labelled release-blocker (pull requests left out)."""
    issues = gh_json("api", f"repos/{REPO}/issues?labels={BLOCKER}&state=open&per_page=100", "--paginate", "--slurp")
    return sorted(issue["number"] for page in issues for issue in page if "pull_request" not in issue)


def smoke_problem(report):
    """Why a release smoke test's report (the game's --smoke-report) fails, or None when the game drew a world."""
    if report is None:
        return "no smoke-test.json: the release smoke test did not run"
    if report.get("error") is not None:
        return f"the smoke run failed: {report['error']}"
    if report.get("frames", 0) < 1500 or not report.get("chunks") or not report.get("meshed_sections") \
            or not report.get("player"):
        return "the smoke run did not draw a world"
    return None


def gate(report, blockers):
    """Why a built game release must not be published, or None: it is published as the latest at once, so its
    smoke test must have passed and no release-blocker may be open."""
    problems = [problem for problem in [smoke_problem(report)] if problem]
    problems += [f"release-blocker #{number} is open" for number in blockers]
    return "; ".join(problems) or None


def last_release(releases, kind):
    keep = is_game_tag if kind == "game" else is_launcher_tag
    ours = [release for release in releases if keep(release["tag_name"])]
    return max(ours, key=lambda release: tag_key(release["tag_name"]), default=None)


def decision(kind, prs, now, paused=False, force=False, new_line=None, blockers=(), daily=False):
    """Whether to release now, and why. prs: the merged pull requests since the last release, each with the
    files it changed under "paths" (player-facing ones only) and when it merged ("merged_at"). new_line: the
    release line ("0.2") when it has no release of this kind yet; its first goes out at once. blockers: the open
    release-blocker issues; they hold every release, forced ones too. daily: the daily run's call for its
    daily-green commit, published without the batch-size and age rules (it comes once a day, with no retry)."""
    counted = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    urgent = [pr for pr in counted if "release-now" in pr["labels"]]
    merged = [pr["merged_at"] for pr in counted if pr.get("merged_at")]
    if blockers:
        return False, "held by " + ", ".join(f"release-blocker #{number}" for number in blockers)
    if force:
        return True, "started by hand with force"
    if paused:
        return False, "RELEASES_PAUSED is set"
    if new_line:
        return True, f"the first release of {new_line}"
    if not counted:
        return False, "no merged pull request changed what players run"
    if urgent:
        return True, "release-now: " + ", ".join(f"#{pr['number']}" for pr in urgent)
    if daily:
        return True, f"{len(counted)} pull requests changed what players run; the daily run passed this commit whole"
    if merged and now - min(merged) >= MAX_WAIT:
        return True, f"{len(counted)} pull requests changed what players run; the oldest has waited {MAX_WAIT.seconds // 3600} hours"
    if len(counted) < BATCH:
        wait = MAX_WAIT - (now - min(merged)) if merged else MAX_WAIT
        return False, (f"{len(counted)} of {BATCH} pull requests: waiting for more, or {wait.seconds // 60 + 1} more "
                       "minutes for the oldest")
    if merged and now - max(merged) < DEBOUNCE:
        wait = DEBOUNCE - (now - max(merged))
        return False, f"{len(counted)} pull requests: waiting {wait.seconds // 60 + 1} more minutes for merges to settle"
    return True, f"{len(counted)} pull requests changed what players run"


# A title's `kind(area): ` prefix (CONTRIBUTING.md "Pull requests") labels the pull request; release names and notes
# read the words after it.
KIND_PREFIX = re.compile(r"^\s*(?:parity|bug|fix|perf|feature|feat|extra|tooling|docs)(?:\([^)]*\))?\s*:\s*", re.I)


def plain(title):
    """A title without its `kind(area): ` prefix, capitalised: "parity(mobs): ghasts fly" -> "Ghasts fly"."""
    rest = KIND_PREFIX.sub("", title.strip(), count=1)
    return rest if rest == title.strip() else rest[:1].upper() + rest[1:]


def clause(title):
    """The start of a pull request title, for a release's name: "Nether portals: vanilla timing" -> "Nether portals"."""
    title = re.sub(r"^(?:fix|fixes|fixed)\s+(?:the\s+)?", "", plain(title), flags=re.I)
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
    title = re.sub(r"\s*\((?:fixes|closes) #\d+\)|,? stacked on #\d+", "", plain(pr["title"])).strip().rstrip(".")
    issues = "".join(f", fixes #{issue}" for issue in pr["issues"])
    return f"- {title} (#{pr['number']}{issues})"


def known_issues(path=ROOT / "docs" / "known-issues.md"):
    if not path.exists():
        return []
    issues = []
    in_bullet = False
    for line in path.read_text().splitlines():
        if line.startswith("- "):
            issues.append(line.rstrip())
            in_bullet = True
        elif in_bullet and line.startswith((" ", "\t")) and line.strip():
            issues[-1] += " " + line.strip()
        else:
            in_bullet = False
    return issues


def line_intro(kind, version, previous):
    """The paragraph a line's first release (X.Y.0) opens with; the first after the dated tags says what changed."""
    found = semver(version, GAME_PREFIX if kind == "game" else LAUNCHER_PREFIX)
    if not found or found[2] != 0 or found[3] is not None:
        return None
    line = f"{found[0]}.{found[1]}"
    if previous and semver(previous, GAME_PREFIX if kind == "game" else LAUNCHER_PREFIX):
        return f"The first release of Horizon {line}."
    if kind == "launcher":
        return (f"The launcher is numbered with the game from now on: this is launcher {line}.0, the release after "
                f"launcher {(previous or '').removeprefix(OLD_LAUNCHER_PREFIX) or '0.16'}. Your launcher takes it by itself.")
    before = (previous or "").removeprefix(GAME_PREFIX)
    return (f"Horizon is numbered from now on: this is Horizon {line}.0" + (f", the release after {before}" if before else "")
            + ". Releases used to be named after the day they came out; from now on they count up from "
            f"{line}.0, and the launcher's too. The launcher installs it by itself once it has updated itself. A server "
            "installed with install.sh that still says it is up to date takes it with "
            f"`horizon-server update --version {version}`.")


def notes(kind, version, prs, issues, previous=None, repository="horizon-rust/horizon"):
    """(title, body) of a release. previous: the release before it, for a line's first release."""
    shown = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    if kind == "game":
        title = f"Horizon {version.removeprefix(GAME_PREFIX)}" + (f": {summary(new, fixed)}" if shown else "")
    else:
        title = f"Horizon launcher {version.removeprefix(OLD_LAUNCHER_PREFIX).removeprefix(LAUNCHER_PREFIX)}"
    fork_launcher = kind == "launcher" and repository == "arismoko/horizon"
    intro = None if fork_launcher else line_intro(kind, version, previous)
    parts = [PLAYERS] if kind == "game" else []
    if fork_launcher:
        title = f"Horizon Fork Launcher (arismoko) {version}"
        parts.append((ROOT / "packaging/fork-launcher-release-notes.md").read_text().replace("{tag}", version).strip())
    if intro:
        parts.append(intro)
    if not shown:
        parts.append("Rebuilt from the latest main.")
    if new:
        parts.append("### New\n" + "\n".join(map(bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(bullet, fixed)))
    if issues and kind == "game":
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != MAINTAINER})
    if contributors:
        parts.append("### Contributors\nThanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    if kind == "launcher" and not fork_launcher:
        parts.append(LAUNCHER_FOOTER)
    if kind == "game":
        parts.append(PLAYERS_FOOTER)
    parts.append("---\n" + DISCLAIMER)
    return title, "\n\n".join(parts) + "\n"


# The workflow that builds and publishes a Nightly. NIGHTLY_CAP is tools/ci/nightly_set.py's MAX_BUILDS_PER_DAY: past
# it, the train waits for the next UTC day unless a pull request is labelled release-now or the run is forced.
NIGHTLY_WORKFLOW = "nightly-build.yml"
NIGHTLY_CAP = 12


def published_nightly():
    """The published Nightly (the `nightly` pre-release) as {tag_name, published_at}, or None."""
    out = run("gh", "api", f"repos/{REPO}/releases/tags/nightly", "--jq", "{tag_name, published_at}", check=False)
    return json.loads(out) if out.strip() else None


SNAPSHOT_RUN = "snapshot build"


def nightly_building():
    """The Nightly's run of `nightly build` under way or waiting, if any: the train starts no second one. A
    snapshot's run (named `snapshot build <commit>` by nightly-build.yml's run-name) has its own concurrency group
    and never holds a Nightly back; every other run of the workflow is a Nightly's."""
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", NIGHTLY_WORKFLOW, "--limit", "20",
                   "--json", "databaseId,status,displayTitle")
    return next((entry["databaseId"] for entry in runs
                 if entry["status"] in ("queued", "in_progress", "waiting", "pending", "requested")
                 and entry["displayTitle"].split(" ", 2)[:2] != SNAPSHOT_RUN.split()), None)


def nightly_builds_today(today):
    """The nightly/build-<date>-<n> branches of `today` (every Nightly build pushes one)."""
    out = run("git", "ls-remote", "origin", f"refs/heads/nightly/build-{today}-*")
    return [line for line in out.splitlines() if line.strip()]


def nightly_decision(release, reason, prs, building, built_today, force=False, cap=NIGHTLY_CAP):
    """The train's decision held back by the Nightly build: one at a time, and at most `cap` a day unless a pull
    request is labelled release-now or the run is forced. Returns (release, reason, urgent)."""
    urgent = force or any("release-now" in pr["labels"] for pr in prs if pr["paths"])
    if not release:
        return False, reason, urgent
    if building:
        return False, f"a Nightly is being built (run {building}): the next decision takes what it leaves", urgent
    if built_today >= cap and not urgent:
        return False, f"{built_today} Nightlies today already, the most a day takes: the next goes out after 00:00 UTC", urgent
    return True, reason, urgent


def gather(kind, head, releases=None):
    """The last published release of this kind and the pull requests merged since, with their player-facing
    paths."""
    releases = published_releases() if releases is None else releases
    last = last_release(releases, kind)
    base = last["tag_name"] if last else run("git", "rev-list", "--max-parents=0", head).split()[0]
    return last, merged_between(kind, base, head)


def merged_between(kind, base, head):
    """The pull requests merged into main between base and head, with their player-facing paths ("paths")."""
    exporter_paths = exporters()
    candidates = merged_pull_requests(base, head)
    if not candidates:
        return []
    # Merge subjects survive imports and PR numbers are repository-local. Fetch this
    # repository's merge identities AND note fields in one fully paginated query.
    owned = merged_pull_request_metadata()
    prs = []
    for number, commit in candidates:
        if (number, commit) not in owned:
            continue
        pr = owned[(number, commit)].copy()
        pr["paths"] = [path for path in changed_paths(commit) if player_facing(path, kind, exporter_paths)]
        prs.append(pr)
    return prs


def passed_on(workflow, sha):
    """Whether the newest finished push run of workflow on main for commit sha passed."""
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", workflow, "--branch", "main", "--event", "push",
                   "--commit", sha, "--limit", "5", "--json", "status,conclusion")
    done = [entry for entry in runs if entry["status"] == "completed" and entry["conclusion"] != "cancelled"]
    return bool(done) and done[0]["conclusion"] == "success"


def gate_jobs_passed(run_id, required):
    """Require every named gate on the latest attempt, regardless of non-gate failures."""
    pages = gh_json("api", "--paginate", "--slurp",
                    f"repos/{REPO}/actions/runs/{run_id}/jobs?filter=latest&per_page=100")
    jobs = [job for page in pages for job in page["jobs"] if job["name"] in required]
    return (required == {job["name"] for job in jobs}
            and all(job["status"] == "completed" and job["conclusion"] == "success" for job in jobs))


DAILY_GREEN = "daily-green"
FORK_GATE = {"lint", "verify", "verify-static", "launcher", "pool-watch"}
SNAPSHOT_GATE = {"lint", "verify", "verify-static", "launcher"}


def daily_green_commit():
    """The commit the tag `daily-green` points to (the newest fully green daily run), freshly fetched, or ""."""
    run("git", "fetch", "--quiet", "--force", "origin", f"+refs/tags/{DAILY_GREEN}:refs/tags/{DAILY_GREEN}", check=False)
    return run("git", "rev-parse", "--verify", "--quiet", f"refs/tags/{DAILY_GREEN}^{{commit}}", check=False).strip()


def fork_gate_passed(sha):
    """Whether the newest dyna-checks main-push run on sha passed every gate job (the snapshot's check)."""
    pages = gh_json("api", "--paginate", "--slurp",
                    f"repos/{REPO}/actions/workflows/dyna-ci.yml/runs?branch=main&event=push&head_sha={sha}&per_page=100")
    runs = [entry for page in pages for entry in page["workflow_runs"]
            if entry["head_sha"] == sha and entry["head_branch"] == "main" and entry["event"] == "push"]
    return bool(runs) and gate_jobs_passed(runs[0]["id"], SNAPSHOT_GATE)


def channel_head(channel, head):
    """The commit a fork channel builds, or exits naming the check it failed."""
    if channel == "nightly":
        green = daily_green_commit()
        if not green:
            raise SystemExit(f"::error::refused: no `{DAILY_GREEN}` tag yet; a nightly ships only a commit whose "
                             "daily run was fully green")
        sha = run("git", "rev-parse", "--verify", "--quiet", f"{head}^{{commit}}", check=False).strip() if head else green
        if sha != green:
            raise SystemExit(f"::error::refused: {head} is not the commit `{DAILY_GREEN}` points to ({green}); "
                             "a nightly ships only the commit of the newest fully green daily run, with its SDK")
        return sha
    sha = run("git", "rev-parse", "--verify", "--quiet", f"{head or 'refs/remotes/origin/main'}^{{commit}}",
              check=False).strip()
    if not sha:
        raise SystemExit(f"::error::refused: {head} is not a commit of this repository")
    if subprocess.run(["git", "merge-base", "--is-ancestor", sha, "refs/remotes/origin/main"], cwd=ROOT).returncode:
        raise SystemExit(f"::error::refused: {sha} is not on main; a snapshot builds only a commit of main")
    if not fork_gate_passed(sha):
        raise SystemExit(f"::error::refused: dyna-checks' gate jobs ({', '.join(sorted(SNAPSHOT_GATE))}) "
                         f"did not all pass on a main push of {sha}; a snapshot needs the gate green on its commit")
    return sha


def build_tag(channel, date, sha):
    """A build's tag and the version it installs as: `nightly-<date>-<short>` or `snapshot-<date>-<short>`
    (horizon_release_sign::Channel parses it; the launcher installs under it)."""
    return f"{channel}-{date}-{sha[:7]}"


def green_main_head(parity=False, eligible=None):
    """The newest commit on main whose checks passed: what an automatic release builds. With parity (game
    releases), the newest commit parity-main passed on whose checks passed too: parity-main runs on batches of
    merges, so the commits a run skipped are covered by the next one, and a superseded run counts for nothing. A
    later batch that failed holds only the merges after that commit, which the green one doesn't contain."""
    if REPO == "arismoko/horizon":
        # Runs are ordered by creation, not completion: a late old run must not
        # displace a newer push. Paginate beyond bursts of cancelled main runs.
        pages = gh_json("api", "--paginate", "--slurp",
                        f"repos/{REPO}/actions/workflows/dyna-ci.yml/runs?branch=main&event=push&per_page=100")
        seen = set()
        for page in pages:
            for entry in page["workflow_runs"]:
                sha = entry["head_sha"]
                if entry["head_branch"] != "main" or entry["event"] != "push" or sha in seen:
                    continue
                seen.add(sha)
                if eligible is not None and not eligible(sha):
                    continue
                # The wasm fixture left main pushes for the daily run (daily-green), so the gate alone counts here.
                if gate_jobs_passed(entry["id"], FORK_GATE):
                    return sha
        return None
    if parity:
        # Only the successful runs: while merges land, one run at a time finishes (the pending one is replaced at
        # every push), and a later failed or timed-out batch must not starve the Nightly of the commit an earlier
        # run passed (2026-11-02: no Nightly for 30 h while main had a green commit parity-main had passed).
        runs = gh_json("run", "list", "--repo", REPO, "--workflow", "parity-main", "--branch", "main", "--event",
                       "push", "--status", "success", "--limit", "20", "--json", "headSha,status,conclusion")
        for entry in runs:
            if entry["status"] == "completed" and entry["conclusion"] == "success" and passed_on("checks", entry["headSha"]):
                return entry["headSha"]
        return None
    # 50, not 10: a busy hour of merges cancels many runs, which pushes the last finished one out of a short window.
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", "checks", "--branch", "main", "--event", "push",
                   "--limit", "50", "--json", "headSha,status,conclusion")
    for entry in runs:
        if entry["status"] == "completed" and entry["conclusion"] != "cancelled":
            return entry["headSha"] if entry["conclusion"] == "success" else None
    return None


def write_outputs(values):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            for name, value in values.items():
                out.write(f"{name}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["decide", "notes", "gate"])
    parser.add_argument("--kind", choices=["game", "launcher"], required=True)
    parser.add_argument("--head", help="the commit to release (default: the newest commit on main whose checks passed)")
    parser.add_argument("--daily", action="store_true",
                        help="decide: the daily run's nightly, without the batch-size and age rules")
    parser.add_argument("--channel", choices=["nightly", "snapshot"],
                        help="decide: the fork's kind of build, which picks and checks the commit (see above)")
    parser.add_argument("--tag", help="notes: the release's tag")
    parser.add_argument("--force", action="store_true", help="release even without player-facing changes, before merges settle, while paused or before parity-main passed")
    parser.add_argument("--out", type=Path, help="notes: write title.txt and body.md here")
    parser.add_argument("--now", help="the time to decide at (ISO 8601; default now)")
    parser.add_argument("--smoke", type=Path, help="gate: the release smoke test's report (smoke-test.json)")
    parser.add_argument("--blockers", action="store_true", help="gate: also check that no release-blocker is open")
    args = parser.parse_args()
    now = dt.datetime.fromisoformat(args.now) if args.now else dt.datetime.now(dt.timezone.utc)

    if args.command == "gate":
        report = json.loads(args.smoke.read_text()) if args.smoke and args.smoke.exists() else None
        problem = gate(report, open_blockers() if args.blockers else [])
        line = f"Not publishing: {problem}" if problem else "Ready to publish: the smoke test passed" + (
            " and no release-blocker is open" if args.blockers else "")
        print(json.dumps(report) if report else "", line, sep="\n")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                summary.write(f"### {line}\n")
        return 1 if problem else 0

    # Game releases ship only a commit the vanilla harnesses passed on (parity-main, docs/release-flow.md).
    parity = args.kind == "game" and args.command == "decide" and not args.force
    head = channel_head(args.channel, args.head) if args.channel else args.head or green_main_head(parity)
    if not head:
        result = {"release": False, "reason": "main's latest checks did not pass" + (
            " or parity-main did not pass on them" if parity else "")}
        print(json.dumps(result, indent=1))
        write_outputs({"release": "false", "reason": result["reason"]})
        return 0
    head = run("git", "rev-parse", head).strip()
    if args.channel == "snapshot" and args.command == "decide":
        # A snapshot is for testing a commit of main, never for players: the gate on its commit is its only rule (no
        # batch, cap, pause or release-blocker), and its tag is its own (never `nightly`).
        tag = build_tag("snapshot", now.date().isoformat(), head)
        result = {"release": True, "reason": f"the gate passed on {head[:9]}: snapshot {tag}", "tag": tag,
                  "head": head, "urgent": False}
        print(json.dumps(result, indent=1))
        write_outputs({"release": "true", "reason": result["reason"], "tag": tag, "head": head, "urgent": "false"})
        return 0
    nightly = published_nightly() if args.kind == "game" else None
    if nightly:
        run("git", "fetch", "--quiet", "--force", "origin", "refs/tags/nightly:refs/tags/nightly")
        last, prs = nightly, merged_between("game", "refs/tags/nightly", head)
    else:
        last, prs = gather(args.kind, head)
    if args.command == "notes":
        version = args.tag or "unreleased"
        previous = last["tag_name"] if last and last["tag_name"] != version else None
        title, body = notes(args.kind, version, prs, known_issues(), previous=previous, repository=REPO)
        if args.out:
            args.out.mkdir(parents=True, exist_ok=True)
            (args.out / "title.txt").write_text(title + "\n")
            (args.out / "body.md").write_text(body)
        print(title + "\n\n" + body)
        return 0

    tags = run("git", "tag", "--list").split()
    line = release_line()
    prefix = GAME_PREFIX if args.kind == "game" else LAUNCHER_PREFIX
    new_line = f"{line[0]}.{line[1]}" if line and first_on_line(tags, prefix, line) and not nightly else None
    blockers = open_blockers() if args.kind == "game" else []
    release, reason = decision(args.kind, prs, now, os.environ.get("RELEASES_PAUSED") == "1", args.force, new_line,
                               blockers, daily=args.daily)
    if release and args.head and parity and not args.channel and not (passed_on("parity-main", head) and passed_on("checks", head)):
        release, reason = False, f"waiting for checks and parity-main to pass on {head[:9]}"
    urgent = False
    if args.kind == "game":
        release, reason, urgent = nightly_decision(release, reason, prs, nightly_building(),
                                                   len(nightly_builds_today(now.date().isoformat())), args.force)
    minor = any("feature" in pr["labels"] for pr in prs if pr["paths"])
    tag = "nightly" if args.kind == "game" else next_launcher_tag(tags, minor, line)
    result = {
        "release": release, "reason": reason, "tag": tag, "head": head, "urgent": urgent,
        "last": last["tag_name"] if last else None, "last_published": last["published_at"] if last else None,
        "pull_requests": [{key: pr[key] for key in ("number", "title", "labels", "paths")} for pr in prs],
    }
    print(json.dumps(result, indent=1))
    write_outputs({"release": str(release).lower(), "reason": reason, "tag": tag, "head": head,
                   "urgent": str(urgent).lower()})
    return 0


if __name__ == "__main__":
    sys.exit(main())
