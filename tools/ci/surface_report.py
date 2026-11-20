"""The mod API surface report: what a pull request changes in what mods can name, and in what wasm mods are checked by.

Exports the surface of the merge base and of the checked-out tree with the head's `horizon-stage`, each with its calls
and core (`--calls`: what the game compares when a wasm mod loads), diffs them, and writes the classified report to the
job summary. It also compares what both builds compiled (`--layouts`): the core layout and every call's hash, as the
game compares them, which `crates/horizon-mod-api/examples/wire_layouts.rs` prints. The head's are built here; the
base's are read from the build folder's store (`--store-layouts`, which only main's pushes fill, one file per commit;
a symlink, another schema, no calls or a call count other than the one written beside them is a miss) or built by checking the base
out in place, with its own copy of the example, then the head again. A base without the example is compared by source
alone, with a warning. A pull request that changes the gate's own code gets a warning line that the result needs a
maintainer's look. The base comes from the event: on `pull_request` the checkout is
GitHub's merge commit, whose first parent is the base; on a dispatch with `pr`, GitHub's compare API gives the merge
base of that pull request's head and its base branch. Only that one commit is fetched (`--depth 1`), and only its
`crates/` and `tools/layering` are unpacked.

It fails for a new change note that names an id that did not change or cannot be read, and, with `gate = true` in
tools/ci/surface.toml (on), for a breaking change no new note names and whenever there is nothing to compare with: the
base cannot be found or fetched, or the head's tool cannot read the base tree or surface (a change to the declaration
format), or this tree's compiled layouts, or those of a base that has the example, cannot be built. With the gate off
those pass, saying so in one line (the compiled layouts missing, it reports without them).

`surface_report.py --store-layouts DIR` builds the checked-out commit's compiled layouts and stores them in DIR.
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
LAYOUTS_EXAMPLE = "crates/horizon-mod-api/examples/wire_layouts.rs"
LAYOUTS_SCHEMA = 1
# Stored compiled layouts kept in the build folder, newest first; one is about 200 KB.
LAYOUTS_KEPT = 300
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


def target_dir() -> Path:
    return Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")


def build_layouts() -> str:
    """The checked-out tree's compiled layouts (the `wire_layouts` example's output)."""
    subprocess.run(["cargo", "build", "--locked", "--quiet", "-p", "horizon-mod-api", "--example", "wire_layouts"],
                   cwd=ROOT, check=True)
    binary = target_dir() / "debug" / "examples" / ("wire_layouts.exe" if os.name == "nt" else "wire_layouts")
    return subprocess.run([str(binary)], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def layout_calls(text: str) -> int | None:
    """How many calls a compiled layouts document lists, or None for one this script cannot read: another schema, no
    calls, or not as many as the count its producer wrote beside them."""
    try:
        document = json.loads(text)
    except ValueError:
        return None
    if not isinstance(document, dict) or document.get("schema") != LAYOUTS_SCHEMA:
        return None
    calls = document.get("calls")
    if not isinstance(calls, list) or not calls or document.get("count") != len(calls):
        return None
    return len(calls)


def stored_layouts(store: Path | None, sha: str) -> str | None:
    """A commit's stored compiled layouts, or None (a miss, so the base is built): none stored, a symlink, or one
    `layout_calls` cannot read. Its calls are not checked against the head's reading of the base tree: a head whose
    generator emits another call would turn a genuine base into a miss, and the compiled diff compares them anyway."""
    if store is None or store.is_symlink():
        return None
    path = store / f"{sha}.json"
    if path.is_symlink() or not path.is_file():
        return None
    try:
        text = path.read_text()
    except OSError:
        return None
    return text if layout_calls(text) else None


def has_example(sha: str, run_git=git) -> bool:
    return run_git("ls-tree", "--name-only", sha, "--", LAYOUTS_EXAMPLE).strip() != ""


def built_base_layouts(sha: str, build=build_layouts, run_git=git) -> str:
    """The base's compiled layouts, built by checking it out here, with its own copy of the example; the head is
    checked out again whatever happens. Refused in a tree with changes, which the checkout would carry or lose."""
    if run_git("status", "--porcelain", "--untracked-files=no").strip():
        raise ValueError("the checkout has changes, so the base cannot be checked out in it")
    head = run_git("rev-parse", "HEAD").strip()
    run_git("checkout", "-q", "--detach", sha)
    try:
        return build()
    finally:
        run_git("checkout", "-q", "-f", "--detach", head)


def compiled_layouts(sha: str, work: Path, store: Path | None, build=build_layouts,
                     run_git=git) -> tuple[bool, str]:
    """Writes both builds' compiled layouts to work (base-layouts.json, head-layouts.json): (whether it did, a line
    saying where the base's came from). A base without the example (based before it) is compared by source alone,
    with a warning. Raises when this tree's cannot be built, or a base that has the example cannot build its own."""
    head = build()
    if layout_calls(head) is None:
        raise ValueError("this tree's wire_layouts printed no readable layouts")
    base, origin = stored_layouts(store, sha), "stored"
    if base is None:
        if not has_example(sha, run_git):
            return False, ("Warning: the base has no wire_layouts example, so only the source is compared; merge main "
                           "for the compiled comparison.\n")
        base, origin = built_base_layouts(sha, build, run_git), "built"
        if layout_calls(base) is None:
            raise ValueError("the base's wire_layouts printed no readable layouts")
    (work / "head-layouts.json").write_text(head)
    (work / "base-layouts.json").write_text(base)
    return True, f"The base's compiled layouts: {origin}.\n"


def store_layouts(store: Path, build=build_layouts, run_git=git) -> Path:
    """Stores the checked-out commit's compiled layouts in store, keeping the newest LAYOUTS_KEPT. Only a push to
    main stores, so no pull request's code can leave a baseline for another."""
    if (os.environ.get("GITHUB_EVENT_NAME"), os.environ.get("GITHUB_REF")) != ("push", "refs/heads/main"):
        raise SystemExit("compiled layouts are stored only by a push to main")
    if store.is_symlink():
        raise SystemExit(f"{store} is a symlink")
    text = build()
    if not layout_calls(text):
        raise SystemExit("wire_layouts printed no readable layouts")
    store.mkdir(parents=True, exist_ok=True)
    path = store / f"{run_git('rev-parse', 'HEAD').strip()}.json"
    path.unlink(missing_ok=True)
    path.write_text(text)
    for old in sorted(store.glob("*.json"), key=lambda f: f.stat().st_mtime, reverse=True)[LAYOUTS_KEPT:]:
        old.unlink()
    return path


# The gate's own code: a pull request that changes it is checked by the code it changes.
GATE_PATHS = ("tools/ci/surface_report.py", "tools/ci/surface.toml", "crates/horizon-mod-surface/",
              "crates/horizon-mod-api-gen/src/bin/horizon-stage.rs", LAYOUTS_EXAMPLE)
WORKFLOW = ".github/workflows/ci.yml"


def gate_steps(workflow: str) -> list[str]:
    """The workflow's steps that run this script."""
    return [step for step in workflow.split("\n      - ") if "tools/ci/surface_report.py" in step]


def gate_changed(sha: str, run_git=git) -> str:
    """A warning line when the pull request changes the gate's own code, else ''."""
    try:
        paths = [path for path in run_git("diff", "--name-only", sha, "HEAD", "--").splitlines()
                 if path.startswith(GATE_PATHS)]
        if gate_steps(run_git("show", f"{sha}:{WORKFLOW}")) != gate_steps(run_git("show", f"HEAD:{WORKFLOW}")):
            paths.append(f"{WORKFLOW} (the surface report's steps)")
    except subprocess.CalledProcessError as error:
        return f"Warning: could not tell whether this pull request changes the gate's own code ({error}).\n"
    if not paths:
        return ""
    return (f"Warning: this pull request changes the gate's own code ({', '.join(paths)}), so this result needs a "
            "maintainer's look.\n")


def last_line(text: str) -> str:
    lines = [line for line in text.splitlines() if line.strip() and not line.startswith("unclassified:")]
    return lines[-1] if lines else "no message"


def report(tool: str, base: Path, head: Path, work: Path, gate_on: bool, layouts=None) -> tuple[int, str]:
    """Exports both trees at once, diffs them; (exit code, report text). `layouts()` writes both builds' compiled
    layouts to work (`compiled_layouts`)."""
    exports = {name: subprocess.Popen([tool, "surface", str(tree), str(work / f"{name}.json"),
                                       "--calls", str(work / f"{name}-calls.json")],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
               for name, tree in (("base", base), ("head", head))}
    results = {name: process.communicate() for name, process in exports.items()}
    # This tree failing is a broken generator, which no other step would catch; the base failing
    # is an older tree this tool cannot read, which is no fault of the pull request.
    if exports["head"].returncode != 0:
        return 1, f"The surface export of this tree failed: {last_line(results['head'][1])}.\n"
    if exports["base"].returncode != 0:
        return unchecked(gate_on, f"the head's horizon-stage cannot read the base tree ({last_line(results['base'][1])})")
    compiled, said = False, ""
    if layouts is not None:
        try:
            compiled, said = layouts()
        except (OSError, subprocess.CalledProcessError, ValueError) as error:
            if gate_on:
                return unchecked(gate_on, f"the compiled layouts could not be built ({error})")
            said = f"No compiled layouts ({error}).\n"
    command = [tool, "surface-diff", str(work / "base.json"), str(work / "head.json"),
               "--calls", str(work / "base-calls.json"), str(work / "head-calls.json"),
               *(["--layouts", str(work / "base-layouts.json"), str(work / "head-layouts.json")] if compiled else []),
               "--notes", str(head / NOTES), "--base-notes", str(base / NOTES), "--json", str(work / "diff.json")]
    if gate_on:
        command.append("--gate")
    diff = subprocess.run(command, capture_output=True, text=True)
    # A base surface of another schema is an unreadable base, like a tree the exporter cannot read.
    base_schema = f"{work / 'base.json'}: surface schema "
    if diff.returncode == 2 and base_schema in diff.stderr:
        return unchecked(gate_on, f"the head's horizon-stage cannot read the base surface ({last_line(diff.stderr)})")
    text = said + diff.stdout + diff.stderr
    return (0 if diff.returncode == 0 else 1), text


def unchecked(gate_on: bool, why: str) -> tuple[int, str]:
    """No comparison ran: a failure with the gate on, which can't pass what it never compared."""
    if gate_on:
        return 1, f"No surface comparison: {why}. The gate fails without one.\n"
    return 0, f"No surface report: {why}.\n"


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
    if sys.argv[1:2] == ["--store-layouts"]:
        print(f"Stored {store_layouts(Path(sys.argv[2]))}")
        return 0
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
            store = os.environ.get("WIRE_LAYOUTS_STORE")
            timed = []

            def layouts():
                began = time.monotonic()
                try:
                    return compiled_layouts(sha, work, Path(store) if store else None)
                finally:
                    timed.append(time.monotonic() - began)
            code, text = report(tool, work / "base", ROOT, work, gate_on, layouts)
            text = gate_changed(sha) + text
            built = f", compiled layouts {timed[0]:.1f}s" if timed else ""
            text += f"Base {sha}; fetch {fetched:.1f}s{built}, total {time.monotonic() - started:.1f}s\n"
        else:
            code, text = unchecked(gate_on, why)
    print(text, end="")
    for line in text.splitlines():
        if line.startswith("Warning: "):
            print(f"::warning::{line.removeprefix('Warning: ')}")
    if summary_path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary_path, "a") as out:
            out.write(summary(text))
    return code


if __name__ == "__main__":
    sys.exit(main())
