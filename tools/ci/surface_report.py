"""The mod API surface report: what a pull request changes in what mods can name.

Exports the surface of the merge base and of the checked-out tree with the head's `horizon-stage`, diffs them, and
writes the classified report to the job summary. The base comes from the event: on `pull_request` the checkout is
GitHub's merge commit, whose first parent is the base; on a dispatch with `pr`, GitHub's compare API gives the merge
base of that pull request's head and its base branch. Only that one commit is fetched (`--depth 1`), and only its
`crates/` and `tools/layering` are unpacked.

It passes when the base cannot be found or fetched, or when the head's tool cannot read
the base tree (a change to the declaration format), saying so in one line. It fails for a new change note that names
an id that did not change or cannot be read, and, with `gate = true` in tools/ci/surface.toml (on), for a breaking
change no new note names.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NOTES = "crates/horizon-mod-vocab/changes"
# GitHub refuses a step summary over 1 MiB; the full report stays in the step log.
# Counted in bytes of UTF-8, frame included.
SUMMARY_LIMIT = 200_000


def gate(config: Path) -> bool:
    value = tomllib.loads(config.read_text())["surface"]["gate"]
    if not isinstance(value, bool):
        raise SystemExit(f"{config}: surface.gate must be true or false")
    return value


def merge_parent(commit_text: str) -> str | None:
    """The first parent of a merge commit (`git cat-file -p` text), or None for any other commit."""
    parents = [line.split()[1] for line in commit_text.splitlines() if line.startswith("parent ")]
    return parents[0] if len(parents) == 2 else None


def base_commit(event: str, pr: str, git, api) -> tuple[str | None, str]:
    """The commit to compare against, or None with the reason."""
    if event == "pull_request":
        parent = merge_parent(git("cat-file", "-p", "HEAD"))
        return (parent, "") if parent else (None, "the checkout is not a pull request's merge commit")
    if event == "workflow_dispatch" and pr:
        # The checked-out revision, not the pull request's head now: a push after the run
        # picked its revision would give a base from another history.
        head = git("rev-parse", "HEAD").strip()
        pull = api(f"pulls/{pr}")
        compare = api(f"compare/{pull['base']['ref']}...{head}")
        return compare["merge_base_commit"]["sha"], ""
    return None, f"no base for a {event or 'local'} run"


def github_api(path: str) -> dict:
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=20) as answer:
        return json.load(answer)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def unpack(sha: str, into: Path) -> None:
    git("fetch", "-q", "--no-tags", "--depth", "1", "origin", sha)
    archive = subprocess.Popen(["git", "archive", sha, "crates", "tools/layering"], cwd=ROOT, stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", str(into)], stdin=archive.stdout, check=True)
    if archive.wait() != 0:
        raise subprocess.CalledProcessError(archive.returncode, "git archive")


def last_line(text: str) -> str:
    lines = [line for line in text.splitlines() if line.strip() and not line.startswith("unclassified:")]
    return lines[-1] if lines else "no message"


def report(tool: str, base: Path, head: Path, work: Path, gate_on: bool) -> tuple[int, str]:
    """Exports both trees at once, diffs them; (exit code, report text)."""
    exports = {name: subprocess.Popen([tool, "surface", str(tree), str(work / f"{name}.json")],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
               for name, tree in (("base", base), ("head", head))}
    results = {name: process.communicate() for name, process in exports.items()}
    # This tree failing is a broken generator, which no other step would catch; the base failing
    # is an older tree this tool cannot read, which is no fault of the pull request.
    if exports["head"].returncode != 0:
        return 1, f"The surface export of this tree failed: {last_line(results['head'][1])}.\n"
    if exports["base"].returncode != 0:
        return 0, f"No surface report: the head's horizon-stage cannot read the base tree ({last_line(results['base'][1])}).\n"
    command = [tool, "surface-diff", str(work / "base.json"), str(work / "head.json"),
               "--notes", str(head / NOTES), "--base-notes", str(base / NOTES), "--json", str(work / "diff.json")]
    if gate_on:
        command.append("--gate")
    diff = subprocess.run(command, capture_output=True, text=True)
    # A base surface of another schema is an unreadable base, like a tree the exporter cannot read.
    base_schema = f"{work / 'base.json'}: surface schema "
    if diff.returncode == 2 and base_schema in diff.stderr:
        return 0, f"No surface report: the head's horizon-stage cannot read the base surface ({last_line(diff.stderr)}).\n"
    text = diff.stdout + diff.stderr
    return (0 if diff.returncode == 0 else 1), text


def summary(text: str) -> str:
    """The report for the job summary, at most SUMMARY_LIMIT bytes of UTF-8 in all."""
    head, foot = "### Mod API surface\n\n```text\n", "```\n"
    # Room for the frame and for the line saying what was cut.
    budget = SUMMARY_LIMIT - len((head + foot).encode()) - 64
    data = text.encode()
    if len(data) > budget:
        kept = data[:budget]
        line_end = kept.rfind(b"\n")
        # A line longer than the budget is cut inside itself, on a character boundary.
        kept = kept[: line_end + 1] if line_end >= 0 else kept
        shown = kept.decode(errors="ignore")
        rest = text[len(shown) :].count("\n")
        lines = "line" if rest == 1 else "lines"
        text = shown.rstrip("\n") + f"\n… cut here; {rest} more {lines} in the step log\n"
    return head + text + foot


def main() -> int:
    started = time.monotonic()
    tool = os.environ["HORIZON_STAGE_TOOL"]
    gate_on = gate(ROOT / "tools/ci/surface.toml")
    with tempfile.TemporaryDirectory(dir=os.environ.get("RUNNER_TEMP")) as directory:
        work = Path(directory)
        try:
            sha, why = base_commit(os.environ.get("EVENT", ""), os.environ.get("PR", ""), git, github_api)
            if sha:
                (work / "base").mkdir()
                unpack(sha, work / "base")
        except (OSError, subprocess.CalledProcessError, KeyError, ValueError) as error:
            sha, why = None, f"the base could not be fetched: {error}"
        if sha:
            fetched = time.monotonic() - started
            code, text = report(tool, work / "base", ROOT, work, gate_on)
            text += f"Base {sha}; fetch {fetched:.1f}s, total {time.monotonic() - started:.1f}s\n"
        else:
            code, text = 0, f"No surface report: {why}.\n"
    print(text, end="")
    if summary_path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary_path, "a") as out:
            out.write(summary(text))
    return code


if __name__ == "__main__":
    sys.exit(main())
