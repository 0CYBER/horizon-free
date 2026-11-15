"""The pull request's merge base and where a sync came from, shared by every gate that takes --base.

`pull_request_base` picks the ref a gate compares HEAD with (the first parent of CI's pull-request merge), and
`sync_provenance` proves a sync's upstream commit. check_layers.py, check_addresses.py and tools/hotspots/ use them.
"""

import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, encoding="utf-8", check=True, cwd=ROOT).stdout


def lines(text):
    return text.count("\n") + (1 if text and not text.endswith("\n") else 0)


def sync_provenance(marker, base, head="HEAD"):
    """Require a freshly verified upstream-main ancestor and its exact merge parent."""
    shallow = git("rev-parse", "--is-shallow-repository").strip()
    if shallow == "true":
        return ["sync baseline provenance needs full history; fetch --unshallow before checking"]
    if marker is None:
        return ["sync baseline provenance needs a sync-baseline header"]
    ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", marker, base], cwd=ROOT,
                              capture_output=True)
    if ancestor.returncode == 0:
        return ["sync baseline SHA is already an ancestor of the base"]
    upstream_url = os.environ.get("HORIZON_UPSTREAM_URL", "https://github.com/horizon-rust/horizon.git")
    try:
        fetched = subprocess.run(
            ["git", "fetch", "--no-tags", "--filter=blob:none", upstream_url,
             "+refs/heads/main:refs/sync/upstream-main"],
            cwd=ROOT, capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return ["cannot verify sync baseline: upstream main fetch timed out; network access is required"]
    if fetched.returncode != 0:
        return ["cannot verify sync baseline: upstream main fetch failed; network access is required: "
                + fetched.stderr.strip()]
    upstream_ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", marker, "refs/sync/upstream-main"],
        cwd=ROOT, capture_output=True)
    if upstream_ancestor.returncode != 0:
        return ["sync baseline SHA is not an ancestor of upstream main"]
    merges = git("rev-list", "--merges", "--parents", f"{base}..{head}")
    if not any(marker in row.split()[1:] for row in merges.splitlines()):
        return ["sync baseline SHA must be a parent of a merge commit in base..HEAD"]
    return []


PULL_REQUEST_MERGE_HELP = ("HEAD is the pull request merged into its base branch (CI's pull_request checkout): "
                           "compare with that merge's first parent, the base branch as it is now")


def pull_request_base(base, merged):
    """(the ref to compare HEAD with, error). With `merged`, HEAD merges the pull request into a descendant of `base`
    (the event's base, where the branch last met main): main has changed since, and the merge holds those changes too,
    so only the merge's own change, against its first parent, is the pull request's. Shared by the gates that take
    --base (check_addresses.py, check_layers.py)."""
    if not merged:
        return base, None
    parents = git("rev-list", "--parents", "-n", "1", "HEAD").split()[1:]
    if len(parents) != 2 or subprocess.run(["git", "merge-base", "--is-ancestor", base, parents[0]],
                                           cwd=ROOT).returncode != 0:
        return base, "--pull-request-merge needs HEAD to merge the pull request into a descendant of --base."
    return parents[0], None


def verified_sync_config(base, config_path):
    """Read an upstream inventory only after verifying a changed sync declaration."""
    try:
        from .check_file_size import parse_baseline
    except ImportError:
        from check_file_size import parse_baseline
    baseline = ROOT / "tools/file_size_baseline.txt"
    marker, _ = parse_baseline(baseline.read_text() if baseline.exists() else "")
    previous = subprocess.run(["git", "show", f"{base}:tools/file_size_baseline.txt"],
                              cwd=ROOT, capture_output=True, text=True)
    old_marker, _ = parse_baseline(previous.stdout if previous.returncode == 0 else "")
    if marker is None or marker == old_marker:
        return None, []
    merge_base = git("merge-base", base, "HEAD").strip()
    errors = sync_provenance(marker, merge_base)
    if errors:
        return None, errors
    source = subprocess.run(["git", "show", f"{marker}:{config_path}"], cwd=ROOT,
                            capture_output=True, text=True)
    if source.returncode != 0:
        return None, [f"verified upstream commit has no {config_path} inventory"]
    config = json.loads(source.stdout)
    return config, []
