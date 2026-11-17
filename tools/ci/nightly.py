#!/usr/bin/env python3
"""The Nightly, the only build Horizon ships (.github/workflows/nightly-build.yml, docs/release-flow.md "Nightly").

    nightly.py pick [--force]      which commit tonight's nightly builds: GitHub outputs and nightly-notes.md
    nightly.py rotate              renames the current `nightly` pre-release to nightly-<its date>, frees the tag
    nightly.py prune [--days 7]    deletes the nightly-<date> pre-releases and tags older than that, nothing else
    nightly.py prune-builds [--days 7]   deletes the nightly/build-<date>-<n> branches older than that, but the
                                   newest and any a published nightly points to (Ludovic, 2026-10-27)
    nightly.py keep-source <sha> <date>  tags the nightly's commit nightly-source/<date>-<short>, for good
    nightly.py sdk-archives        the assets the SDK's packing legs make (tools/prebuilt_game.py gather), one per line
    nightly.py sdk-assets          every asset a complete sdk-<sha> release holds: those, sdk.json and the signed sums
    nightly.py sdk-index <sha> <out>     writes sdk-<sha>'s sdk.json to <out>, checked against its SHA256SUMS
    nightly.py sdk-previous <sha> <out>  prints the SDK published before <sha> on its history, and writes its
                                   surface.json to <out>, checked; prints nothing when there is none

A nightly builds main plus the pull requests the trusted reviewers picked (the set: tools/ci/nightly_set.py), merged
in their order and pushed as nightly/build-<date>-<n>; with no set, main alone. Main is its newest commit where
this repository's checks passed (`checks` and `parity-main`), as for a release
(release_train.green_main_head), or NIGHTLY_BASE when the caller gives that commit. It is published as the
`nightly` pre-release; the previous one stays a week as nightly-<date>.
On horizon-rust/horizon the Nightly is exactly main's commit (PR picks are refused), and the SDK built for that commit
(`sdk-<sha>`, published first by nightly-build.yml) is its pair: its sdk.json goes into the Nightly's signed sums.
Its notes list what reached main since the previous nightly, then the picked pull requests. Runs from main's
checkout with `gh`; it reads and merges the branches with git and never runs anything from them.

Each nightly ships its exact source: horizon-source.tar.gz (`git archive` of the commit, signed with the builds) on
the release, and the tag nightly-source/<date>-<short>, which no prune deletes. Native mods pin a nightly's commit
(docs/native-mods.md), so that commit must stay downloadable after its release and build branch are gone.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

if __package__:  # imported as tools.ci.nightly (the tests)
    from . import nightly_set as ns
    from .. import release_train as train
else:  # run as tools/ci/nightly.py: its folder is on the path
    import nightly_set as ns
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import release_train as train

DATED = re.compile(r"^nightly-(\d{4}-\d{2}-\d{2})$")
TITLE_DATE = re.compile(r"Nightly (\d{4}-\d{2}-\d{2})")
SOURCE_TAG = "nightly-source/"


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


def output(**values: str) -> None:
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as f:
        for k, v in values.items():
            f.write(f"{k}={v}\n")


PLAYERS = ("The launcher installs it by itself; your worlds stay as they are. A Nightly can include pull requests "
           "the Team picked before they are merged (below). Report bugs with the version the game shows "
           "(\"Nightly <date> · <commit>\").")


def source_tag(today: str, sha: str) -> str:
    """The permanent tag of a nightly's commit: neither `prune` (nightly-<date> only) nor `prune-builds` touches it."""
    return f"{SOURCE_TAG}{today}-{sha[:7]}"


def source_line(today: str, sha: str) -> str:
    return (f"Source: commit {sha}, kept for good as the tag `{source_tag(today, sha)}`; horizon-source.tar.gz is "
            "its tree. Native mods built for this Nightly pin that commit.")


def notes(today: str, chosen: Ref, main: Ref, picked: list[dict], left_out: list[str], merged: list[dict],
          issues: list[str]) -> str:
    """A nightly's notes: what reached main since the previous nightly (New, Fixed, as a release's notes), then the
    reviewers' picks, the known issues and the thanks."""
    shown = [pr for pr in merged if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    built = f"main ({main.sha[:7]})" + (f" with {len(picked)} pull requests merged in" if picked else "")
    # The banner first: people who land on the Nightly ran its game zip for the launcher (a player, 2026-10-30).
    parts = [train.PLAYERS, f"Nightly {today} · {chosen.sha[:7]}: {built}, built from `{chosen.name}` ({chosen.sha}).",
             source_line(today, chosen.sha), PLAYERS]
    if not shown and not picked:
        parts.append("Nothing new for players on main since the previous Nightly.")
    if new:
        parts.append("### New\n" + "\n".join(map(train.bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(train.bullet, fixed)))
    if picked:
        parts.append(f"### Not merged yet ({len(picked)})\n" + "\n".join(
            f"- {ns.plain(e['title'])} (#{e['number']}) by {e['author']}, at {e['head'][:7]}, picked by {e['asked_by']}"
            for e in picked))
    if left_out:
        parts.append(f"### Left out ({len(left_out)})\n" + "\n".join(f"- {ns.plain(why)}" for why in left_out))
    if issues:
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != train.MAINTAINER})
    if contributors:
        parts.append("Thanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    parts.append(train.DISCLAIMER)
    return "\n\n".join(parts) + "\n"


def merged_since(previous: str | None, main: str) -> list[dict]:
    """The pull requests merged into main since the previous nightly (its commit holds the main it was built on)."""
    if not previous or subprocess.run(["git", "cat-file", "-e", f"{previous}^{{commit}}"]).returncode != 0:
        return []
    return train.merged_between("game", previous, main)


def refresh(entries: list[dict], get, patterns: list[str]) -> tuple[list[dict], list[str]]:
    """The scheduled nightly's set: each pull request still at the head it was picked at (a push after the pick
    waits for someone to pick it again); merged, closed and refused ones are out."""
    kept, left_out = [], []
    for e in entries:
        pr = get(e["number"])
        if pr is not None and pr.state == "merged":
            left_out.append(f"#{e['number']} {e['title']}: merged, so it's in main")
        elif why := ns.check(pr, e["number"], patterns):
            left_out.append(f"{why}: left out")
        else:
            kept.append(e)
    return kept, left_out


def requested(spec: str, get) -> list[dict]:
    """A dry run's set from its input: `2640@<head> 2642@<head>`."""
    prs = []
    for word in spec.split():
        number, _, head = word.partition("@")
        pr = get(int(number))
        prs.append({"number": int(number), "head": head or (pr.head if pr else ""), "title": pr.title if pr else "?",
                    "author": pr.author if pr else "?", "asked_by": "dry run"})
    return prs


def stop(repo: str, issue: str, text: str) -> None:
    """Says why the build stopped, where it was asked for, and fails the job."""
    print(text)
    if issue and repo:
        ns.answer(repo, issue, text)
    sys.exit(1)


def build_set(main: Ref, prs: list[dict], left_out: list[str], today: str, force: bool, dry_run: bool) -> None:
    """main + the set, merged with git only (nightly_set.assemble), pushed as nightly/build-<date>-<n>."""
    repo, issue = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("NIGHTLY_ISSUE", "")
    token, event = os.environ.get("GH_TOKEN", ""), os.environ.get("GITHUB_EVENT_NAME", "")
    patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
    gone = set(ns.fetch_heads([(e["number"], e["head"]) for e in prs]))
    kept = []
    for e in prs:
        if e["number"] in gone:
            left_out.append(f"#{e['number']} {e['title']}: its head {e['head'][:7]} is gone; ask for it again")
        elif bad := [f for f in ns.changed_files(main.sha, e["head"]) if ns.guarded(f, patterns)]:
            left_out.append(f"#{e['number']} {e['title']}: changes `{bad[0]}`, which only Ludovic merges")
        else:
            kept.append(e)
    try:
        sha = ns.assemble(main.sha, [(e["number"], e["head"]) for e in kept])
    except ns.Conflict as c:
        state, _, _ = ns.load_state()
        asked_in = issue or next((x["issue"] for x in reversed(state.get("log", [])) if x.get("issue")), "")
        stop(repo, asked_in, f"the build stopped: {c.text()}")
    if repo == "horizon-rust/horizon" and sha != main.sha:
        stop(repo, issue, f"the build stopped: final source {sha} is not main's commit {main.sha}, the one an SDK is built for.")
    previous = git("ls-remote", "--tags", "origin", "refs/tags/nightly").split("\t")[0] or None
    build = force or previous != sha
    name = "(unchanged)"
    if build:
        if dry_run:
            run, attempt = os.environ.get("GITHUB_RUN_ID", ""), os.environ.get("GITHUB_RUN_ATTEMPT", "")
            if not re.fullmatch(r"[0-9]+", run) or not re.fullmatch(r"[0-9]+", attempt):
                stop(repo, "", "a dry run requires its workflow run and attempt identity")
            # Temporary checkout transport, never a real build or saved selection.
            # The workflow cleans this isolated ref even if pick fails after pushing it.
            name = f"nightly/dry-run-{run}-{attempt}"
        else:
            done = ns.builds_today(today)
            if len(done) >= ns.MAX_BUILDS_PER_DAY and event != "schedule" and os.environ.get("NIGHTLY_URGENT") != "true":
                stop(repo, issue, f"{len(done)} Nightly builds today already, the most a day takes: the next scheduled "
                                  "one (20:17 UTC) builds the set.")
            name = ns.next_build(today, done)
        if not ns.push(sha, name, token):
            stop(repo, issue, f"could not push {name}")
        record = {"ref": name, "sha": sha, "main": main.sha, "at": dt.datetime.now(dt.timezone.utc).isoformat(
            timespec="seconds"), "event": event, "dry_run": dry_run, "run": os.environ.get("GITHUB_RUN_ID", ""),
            "prs": [{k: e[k] for k in ("number", "head", "title", "author", "asked_by")} for e in kept],
            "left_out": left_out}
        if not dry_run:
            ns.update_state(lambda st, builds: (st, builds + json.dumps(record) + "\n", f"build {name}"), token)
    chosen = Ref(name, sha, dt.datetime.now(dt.timezone.utc))
    with open("nightly-notes.md", "w") as f:
        f.write(notes(today, chosen, main, kept, left_out, merged_since(previous, main.sha), train.known_issues()))
    output(build=str(build).lower(), sha=sha, short=sha[:7], source=name, date=today, version=f"nightly-{today}",
           nightly=f"{today} {sha[:7]}", source_tag=source_tag(today, sha))
    print(f"{'build' if build else 'skip (unchanged since the last nightly)'}: {name} {sha}, main {main.sha[:7]}"
          + "".join(f" + #{e['number']}" for e in kept))


def package_format() -> int:
    """The prebuilt package's layout number, from its one place; anything but a number fails."""
    return int((Path(__file__).resolve().parents[2] / "crates/horizon-mod-package/src/sdk/package-format").read_text().rstrip())


# The SDK's packing legs (prebuilt-game-build.yml's matrix): each target, and the Linux server's side.
SDK_LEGS = (("x86_64-unknown-linux-gnu", ""), ("x86_64-unknown-linux-gnu", "-server"),
            ("x86_64-pc-windows-gnu", ""), ("aarch64-apple-darwin", ""))
# The SDK's assets beside its index: each packing leg's three slices, one build data and surface (gathered once,
# tools/prebuilt_game.py gather) and the API source (tools/sdk_api_source.py). What the publish job refuses to
# release without (`nightly.py sdk-archives`), and what a mod tool's fetch needs.
SDK_ARCHIVES = tuple(f"{kind}-{target}{side}.tar.xz" for target, side in SDK_LEGS
                     for kind in ("runtime", "api", "engine")) + ("api-source.tar.xz", "build-data.tar.xz", "surface.json")
# Beside them in the release: the index and the signed sums (`horizon_mod_package::sdk::BESIDE_ASSETS`).
SDK_INDEX = "sdk.json"
SDK_ASSETS = SDK_ARCHIVES + (SDK_INDEX, "SHA256SUMS", "SHA256SUMS.sig")


def ready_sdk_shas() -> set[str]:
    """Snapshot of the complete published `sdk-<sha>` releases; API failures fail closed."""
    releases = train.gh_json("api", "--paginate", "--slurp",
                             f"repos/{train.REPO}/releases?per_page=100")
    return {release["tag_name"].removeprefix("sdk-")
            for page in releases for release in page
            if not release["draft"] and re.fullmatch(r"sdk-[0-9a-f]{40}", release["tag_name"])
            and set(SDK_ASSETS) <= {asset["name"] for asset in release["assets"]}}


def sdk_ready(sha: str) -> bool:
    """A fresh check, never cached across selection and publication."""
    return sha in ready_sdk_shas()


def listed(name: str, data: bytes, sums: str) -> bool:
    """Whether `data` is the `name` that a `sha256sum` listing `sums` names."""
    for line in sums.splitlines():
        digest, _, file = line.partition(" ")
        if file.strip().lstrip("*") == name:
            return digest == hashlib.sha256(data).hexdigest()
    return False


def signature_check(sha: str, folder: Path, names: list[str]) -> list[str]:
    """The command that checks sdk-<sha>'s SHA256SUMS signature in `folder` as the launcher does (horizon-release-sign,
    with this repository's keys and the tag the signature must name), and the bytes of `names` against it."""
    return ["cargo", "run", "--locked", "--quiet", "-p", "horizon-release-sign", "--", "verify",
            "--repository", train.REPO, "--tag", f"sdk-{sha}",
            *(argument for name in names for argument in ("--only", name)), str(folder)]


def released(sha: str, name: str) -> bytes:
    """The asset `name` of sdk-<sha>, once its SHA256SUMS signature is this repository's for `sdk-<sha>` and its bytes match
    them ([`signature_check`])."""
    with tempfile.TemporaryDirectory() as folder:
        gh("release", "download", f"sdk-{sha}", "--repo", train.REPO, "--dir", folder,
           "--pattern", name, "--pattern", "SHA256SUMS", "--pattern", "SHA256SUMS.sig")
        data = (Path(folder) / name).read_bytes()
        if not listed(name, data, (Path(folder) / "SHA256SUMS").read_text()):
            raise SystemExit(f"sdk-{sha}'s {name} does not match its SHA256SUMS")
        checked = subprocess.run(signature_check(sha, Path(folder), [name]), cwd=Path(__file__).resolve().parents[2],
                                 capture_output=True, text=True)
        if checked.returncode != 0:
            raise SystemExit(f"sdk-{sha}'s signature does not check: {checked.stderr.strip()}")
        return data


def checked_index(sha: str, index: bytes) -> dict:
    """sdk-<sha>'s sdk.json, once it names this build, this package format and every asset the release must hold;
    anything else stops the Nightly, naming why."""
    try:
        document = json.loads(index)
        commit, layout, assets = document["build"]["commit"], document["package_format"], set(document["assets"])
    except (ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} is not an SDK index: {error}")
    if commit != sha:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} is for build {commit}")
    if layout != package_format():
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} has package format {layout}, not this source's {package_format()}")
    missing = sorted(set(SDK_ARCHIVES) - assets)
    if missing:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} does not list {', '.join(missing)}")
    return document


def published_index(sha: str) -> bytes:
    """sdk-<sha>'s sdk.json as published, checked ([`released`], [`checked_index`])."""
    index = released(sha, SDK_INDEX)
    checked_index(sha, index)
    return index


def previous_sdk(sha: str) -> str:
    """The build id of the newest complete SDK published before `sha` on its history (the fewest commits behind
    it), or "" when there is none: what this build's surface is compared with."""
    behind = {}
    for other in ready_sdk_shas() - {sha}:
        if is_ancestor(other, sha):
            behind[other] = int(git("rev-list", "--count", f"{other}..{sha}"))
    return min(behind, key=behind.get) if behind else ""


def require_sdk(sha: str) -> None:
    """Refuse before rotating/publishing Nightly if its SDK disappeared or its index is not this build's."""
    if not sdk_ready(sha):
        raise SystemExit(f"Nightly publication refused: {sha} has no complete published sdk-{sha} release")
    published_index(sha)


def green_base(repo: str, issue: str) -> Ref:
    """The main commit a nightly builds on: NIGHTLY_BASE, else main's newest commit where checks and parity-main
    passed (what releases ship), so a merge that breaks main never reaches a nightly: it builds on the commit before."""
    sha = os.environ.get("NIGHTLY_BASE", "").strip() or train.green_main_head(parity=True)
    if not sha:
        stop(repo, issue, "the build stopped: no recent commit of main passed both checks and parity-main, so "
                          "none is green to build on. The next build after they pass takes it.")
    if not is_ancestor(sha, "refs/remotes/origin/main"):
        stop(repo, issue, f"the build stopped: {sha[:9]} is not on main.")
    sha = git("rev-parse", sha)
    return Ref("main", sha, dt.datetime.fromisoformat(git("show", "-s", "--format=%cI", sha)))


def pick(force: bool) -> None:
    refs = ["+refs/heads/main:refs/remotes/origin/main"]
    # A new repository has no Nightly yet. Only fetch the tag when the remote advertises it;
    # main's fetch and every network error still fail normally.
    if git("ls-remote", "--tags", "origin", "refs/tags/nightly").strip():
        refs.append("+refs/tags/nightly:refs/tags/nightly")
    git("fetch", "--quiet", "origin", *refs)
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    repo, token = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GH_TOKEN", "")
    main = green_base(repo, os.environ.get("NIGHTLY_ISSUE", ""))
    dry_run = os.environ.get("DRY_RUN") == "true"
    if repo == "horizon-rust/horizon":
        if os.environ.get("NIGHTLY_PRS", "").strip():
            stop(repo, os.environ.get("NIGHTLY_ISSUE", ""), "a Nightly builds main's commit, the one its SDK is built for, not PR picks")
        return build_set(main, [], [], today, force, dry_run)
    if spec := os.environ.get("NIGHTLY_PRS", "").strip():  # a dry run's own set; the saved one stays
        return build_set(main, requested(spec, ns.lookup(repo)), [], today, force, dry_run)
    state, _, _ = ns.load_state()
    prs, left_out = state.get("prs", []), []
    if prs and os.environ.get("GITHUB_EVENT_NAME") == "schedule" and not dry_run:
        patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
        get = ns.lookup(repo)

        def fresh(st: dict, builds: str):
            kept, out = refresh(st.get("prs", []), get, patterns)
            left_out[:] = out
            if kept == st.get("prs", []):
                return None
            return {**st, "prs": kept}, builds, "the scheduled nightly drops the merged, closed and refused"
        prs = ns.update_state(fresh, token).get("prs", [])
    build_set(main, prs, left_out, today, force, dry_run)


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


BUILD = re.compile(r"^nightly/build-(\d{4}-\d{2}-\d{2})-(\d+)$")


def stale_builds(branches: dict[str, str], published: set[str], today: dt.date, days: int) -> list[str]:
    """The nightly/build-<date>-<n> branches (name -> commit) to delete: older than `days`, never the newest build,
    never one whose commit a published nightly release points to, never a branch not named like a build."""
    builds = {name: sha for name, sha in branches.items() if BUILD.match(name)}
    if not builds:
        return []
    order = lambda name: (BUILD.match(name).group(1), int(BUILD.match(name).group(2)))  # noqa: E731
    newest = max(builds, key=order)
    cutoff = today - dt.timedelta(days=days)
    return sorted(name for name, sha in builds.items() if name != newest and sha not in published
                  and dt.date.fromisoformat(BUILD.match(name).group(1)) < cutoff)


def remote_refs(pattern: str) -> dict[str, str]:
    """origin's refs matching `pattern`: short name -> commit (annotated tags peeled)."""
    refs: dict[str, str] = {}
    for line in git("ls-remote", "origin", pattern).splitlines():
        sha, ref = line.split("\t", 1)
        name = ref.removeprefix("refs/heads/").removeprefix("refs/tags/")
        if name.endswith("^{}"):
            refs[name[:-3]] = sha  # the peeled commit wins over the tag object
        else:
            refs.setdefault(name, sha)
    return refs


def prune_builds(days: int) -> None:
    """Deletes old nightly/build-* branches on origin through the API, one line per deletion (the run summary)."""
    branches = remote_refs("refs/heads/nightly/build-*")
    tags = {name: sha for name, sha in remote_refs("refs/tags/nightly*").items()
            if name == "nightly" or DATED.match(name)}
    today = dt.datetime.now(dt.timezone.utc).date()
    doomed = stale_builds(branches, set(tags.values()), today, days)
    for name in doomed:
        gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/heads/{name}")
        print(f"- deleted `{name}` ({branches[name][:7]})")
    print(f"{len(doomed)} of {len(branches)} nightly/build-* branches deleted (kept: the newest, the published "
          f"nightlies' and the last {days} days')")


def keep_source(sha: str, today: str) -> None:
    """Tags the published nightly's commit for good (source_tag); a rerun of the same build finds it there."""
    tag = source_tag(today, sha)
    there = remote_refs(f"refs/tags/{tag}").get(tag)
    if there == sha:
        print(f"`{tag}` already names {sha[:7]}")
        return
    if there:
        raise SystemExit(f"`{tag}` names {there[:7]}, not {sha[:7]}")
    gh("api", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs", "-f", f"ref=refs/tags/{tag}", "-f", f"sha={sha}")
    print(f"tagged {sha[:7]} `{tag}`")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick").add_argument("--force", action="store_true")
    sub.add_parser("rotate")
    sub.add_parser("require-sdk").add_argument("sha")
    sub.add_parser("sdk-archives")
    sub.add_parser("sdk-assets")
    index = sub.add_parser("sdk-index")
    index.add_argument("sha")
    index.add_argument("out", type=Path)
    previous = sub.add_parser("sdk-previous")
    previous.add_argument("sha")
    previous.add_argument("out", type=Path)
    sub.add_parser("prune").add_argument("--days", type=int, default=7)
    sub.add_parser("prune-builds").add_argument("--days", type=int, default=7)
    keep = sub.add_parser("keep-source")
    keep.add_argument("sha")
    keep.add_argument("date")
    args = ap.parse_args()
    if args.cmd == "pick":
        pick(args.force)
    elif args.cmd == "require-sdk":
        require_sdk(args.sha)
    elif args.cmd == "sdk-archives":
        print("\n".join(SDK_ARCHIVES))
    elif args.cmd == "sdk-assets":
        print("\n".join(SDK_ASSETS))
    elif args.cmd == "sdk-index":
        args.out.write_bytes(published_index(args.sha))
    elif args.cmd == "sdk-previous":
        if build := previous_sdk(args.sha):
            args.out.write_bytes(released(build, "surface.json"))
            print(build)
    elif args.cmd == "rotate":
        rotate()
    elif args.cmd == "prune-builds":
        prune_builds(args.days)
    elif args.cmd == "keep-source":
        keep_source(args.sha, args.date)
    else:
        prune(args.days)


if __name__ == "__main__":
    main()
