#!/usr/bin/env python3
"""Issues labelled `needs-info` close 7 days after the label went on, unless someone answered (docs/release-flow.md
"Trusted reviewers").

    needs_info.py [--apply]     prints what it would do; --apply does it (needs-info.yml)

For each open issue with the label, from the newest time it was added:
- a comment after that by a person (not a bot, so not the triage command's confirmation) takes the label off;
- with no such comment after 7 days, the issue gets one comment and closes as not planned.
Other labels, edits and bot comments change nothing. Pull requests are skipped. It runs nothing from any issue.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.parse
from datetime import datetime, timedelta, timezone

LABEL = "needs-info"
DAYS = 7
CLOSING = (f"Closing: no answer in {DAYS} days after we asked for more information. Comment with the missing details "
           "and it can be reopened.")


def gh(*args: str) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:2])}: {r.stderr.strip()[:300]}")
    return r.stdout.strip()


def lines(raw: str) -> list:
    return [json.loads(line) for line in raw.splitlines() if line.strip()]


class Api:
    def __init__(self, repo: str):
        self.repo = repo

    def labelled(self) -> list[int]:
        q = urllib.parse.quote(LABEL, safe="")
        return [n for n, is_pr in lines(gh("api", "--paginate", f"repos/{self.repo}/issues?state=open&labels={q}"
                                           "&per_page=100", "--jq", ".[] | [.number, (.pull_request != null)]"))
                if not is_pr]

    def labelled_at(self, n: int) -> str | None:
        times = lines(gh("api", "--paginate", f"repos/{self.repo}/issues/{n}/events?per_page=100", "--jq",
                         f'.[] | select(.event == "labeled" and .label.name == "{LABEL}") | .created_at | tojson'))
        return max(times) if times else None

    def comments_since(self, n: int, since: str) -> list[dict]:
        return lines(gh("api", "--paginate", f"repos/{self.repo}/issues/{n}/comments?since={since}&per_page=100",
                        "--jq", ".[] | {at: .created_at, bot: (.user.type == \"Bot\")}"))

    def has_label(self, n: int) -> bool:
        return LABEL in gh("api", f"repos/{self.repo}/issues/{n}/labels", "--jq", ".[].name").splitlines()

    def unlabel(self, n: int) -> None:
        gh("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/labels/{urllib.parse.quote(LABEL, safe='')}")

    def close(self, n: int) -> None:
        # Closed first: a failure between the two leaves a closed issue without the note, never a second note
        # tomorrow (the bot's own comments don't count as answers, so a repeat would repeat daily).
        gh("issue", "close", str(n), "-R", self.repo, "--reason", "not planned")
        gh("issue", "comment", str(n), "-R", self.repo, "--body", CLOSING)


def decide(labelled_at: str | None, comments: list[dict], now: datetime) -> str | None:
    """'unlabel', 'close' or None (wait). Timestamps are GitHub's ISO strings (UTC, `Z`)."""
    if labelled_at is None:
        return None
    if any(c["at"] > labelled_at and not c["bot"] for c in comments):
        return "unlabel"
    since = datetime.fromisoformat(labelled_at.replace("Z", "+00:00"))
    return "close" if now - since >= timedelta(days=DAYS) else None


def run(api, apply: bool, now: datetime) -> list[str]:
    done = []
    for n in api.labelled():
        at = api.labelled_at(n)
        action = decide(at, api.comments_since(n, at) if at else [], now)
        # The listing can be minutes old: someone may have taken the label off since.
        if action is None or not api.has_label(n):
            continue
        done.append(f"#{n} {action}")
        if apply:
            api.unlabel(n) if action == "unlabel" else api.close(n)
    return done


if __name__ == "__main__":
    rest = sys.argv[1:]
    if rest not in ([], ["--apply"]):
        sys.exit(__doc__)
    for line in run(Api(os.environ.get("REPO", "horizon-rust/horizon")), rest == ["--apply"],
                    datetime.now(timezone.utc)):
        print(line)
