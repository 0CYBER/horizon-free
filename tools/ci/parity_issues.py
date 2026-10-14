#!/usr/bin/env python3
"""Turns the nightly audit's results into GitHub issues (.github/workflows/nightly.yml).

    python3 tools/ci/parity_issues.py --since 2026-10-15T03:00:00 [--dry-run]

Reads tools/parity.py's state (.local/parity-runner/state.json) for the harnesses that finished since --since,
shards merged into their harness, and compares them with the previous audit (.local/parity-runner/audit.json):

- a harness that fails now and passed in the previous audit gets one open issue labelled parity-regression,
  titled "Parity regression: <harness>" (an open one is commented on instead of opening another);
- a harness with an open issue that still fails gets a comment on it;
- a harness with an open issue that passes again gets its issue closed.

The first audit only records the results. Needs the gh CLI; --dry-run prints what it would do.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / ".local" / "parity-runner" / "state.json"
AUDIT = ROOT / ".local" / "parity-runner" / "audit.json"
LOGS = ROOT / ".local" / "parity-runner" / "logs"
LABEL = "parity-regression"
TITLE = "Parity regression: "


def results(state, since):
    """{harness: "pass" | "fail"} of the harnesses that finished since `since`; a harness fails if a shard does."""
    found = {}
    for name, record in state.get("harnesses", {}).items():
        if record.get("finished", "") < since or record.get("shared"):
            continue
        harness = name.split("#")[0]
        found[harness] = "fail" if record["status"] != "pass" or found.get(harness) == "fail" else "pass"
    return found


def plan(now, previous, open_issues):
    """The actions to take: (kind, harness) with kind open, comment or close. open_issues: {harness: number}."""
    actions = []
    for harness, status in sorted(now.items()):
        if status == "fail" and harness in open_issues:
            actions.append(("comment", harness))
        elif status == "fail" and previous.get(harness) == "pass":
            actions.append(("open", harness))
        elif status == "pass" and harness in open_issues:
            actions.append(("close", harness))
    return actions


def log_tail(harness, lines=60):
    logs = sorted(LOGS.glob(f"{harness}*.log"))
    text = "\n".join(path.read_text(errors="replace") for path in logs)
    return "\n".join(text.splitlines()[-lines:])


def gh(*arguments, dry_run=False):
    if dry_run:
        print("would run: gh " + " ".join(arguments[:4]) + " ...")
        return ""
    return subprocess.run(["gh", *arguments], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--since", required=True, help="the audit's start (the state's local time format)")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    now = results(json.loads(STATE.read_text()), args.since)
    previous = json.loads(AUDIT.read_text()) if AUDIT.exists() else None
    listed = json.loads(gh("issue", "list", "--label", LABEL, "--state", "open", "--limit", "200",
                           "--json", "number,title") or "[]")
    open_issues = {issue["title"][len(TITLE):]: issue["number"] for issue in listed if issue["title"].startswith(TITLE)}
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    run = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{os.environ.get('GITHUB_REPOSITORY', '')}" \
          f"/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
    failing = sorted(harness for harness, status in now.items() if status == "fail")
    print(f"{len(now)} harnesses ran: {len(now) - len(failing)} passed, {len(failing)} failed"
          + (f" ({', '.join(failing)})" if failing else ""))
    if previous is None:
        print("First audit: results recorded, no issues opened.")
        actions = plan(now, {}, open_issues)
    else:
        actions = plan(now, previous, open_issues)
    for kind, harness in actions:
        details = (f"Nightly audit of `{commit[:12]}` ({run}).\n\n"
                   f"Reproduce: `python3 tools/parity.py {harness}`\n\n<details><summary>End of the log</summary>\n\n"
                   f"```\n{log_tail(harness)}\n```\n</details>\n")
        print(f"{kind}: {harness}")
        if kind == "open":
            gh("issue", "create", "--label", LABEL, "--title", TITLE + harness, "--body",
               f"`{harness}` passed in the previous nightly audit and fails now.\n\n" + details, dry_run=args.dry_run)
        elif kind == "comment":
            gh("issue", "comment", str(open_issues[harness]), "--body", "Still failing.\n\n" + details, dry_run=args.dry_run)
        else:
            gh("issue", "close", str(open_issues[harness]), "--comment",
               f"Passes again in the nightly audit of `{commit[:12]}` ({run}).", dry_run=args.dry_run)
    if not args.dry_run:
        merged = dict(previous or {})
        merged.update(now)
        AUDIT.write_text(json.dumps(merged, indent=1, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
