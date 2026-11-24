#!/usr/bin/env python3
"""Pull requests left waiting on their author get one notice, then close (docs/release-flow.md "Stale pull requests").

    stale_prs.py [--dry-run | --apply] [--conflict-days 4] [--changes-days 7] [--grace-days 3] [--max-actions 30]
    stale_prs.py create [--apply]      creates the labels the repository lacks

Prints what it would do; only --apply writes (stale-prs.yml, daily). An open, non-draft pull request to main is
waiting on its author when
- it conflicts with main and nothing happened on it for 4 days (no commit, push, reopen, ready-for-review or
  person's comment), or
- changes were requested and its head is still the commit that review looked at, 7 days after the review.
It gets the `stale` label and one comment naming the reason, with the head it saw in a hidden marker. Any push, a
person's comment, a reopen or the reason going away takes the label off. Still unchanged 3 days after the notice, it
is closed with a comment; reopening it starts again.

Never touched: drafts, pull requests in the merge queue or to another base, and the labels in PROTECTED. A
pull request whose mergeable state GitHub hasn't worked out (UNKNOWN) waits for the next run. A label without the
bot's marker (a person added it) is left alone. Each pull request is read again by itself right before it closes.

It reads the API only: no pull request's code is checked out or run.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse
from datetime import datetime, timedelta, timezone

LABEL = "stale"
PROTECTED = {"keep-open", "pinned", "proposal", "tracker", "release-now"}
BASE = "main"
BOT = "github-actions"  # the workflow token's comments; only they carry a trusted marker
MARKER = re.compile(r"<!-- stale-prs: reasons=([a-z,]+) head=([0-9a-f]{40}) -->")
CONFLICT, CHANGES = "conflict", "changes"
PAGE = 50
WRITE_PAUSE_S = 1
# GitHub works out `mergeable` only when asked, and forgets it whenever main moves.
MERGEABLE_WAIT_S = 150
READ_ATTEMPTS = 4
READ_RETRY_S = 5
GUIDE = "https://github.com/{repo}/blob/main/docs/release-flow.md#stale-pull-requests"

# A review counts as a comment: a reply in a review thread is a review of its own.
ACTIVITY = """timelineItems(itemTypes: [PULL_REQUEST_COMMIT, HEAD_REF_FORCE_PUSHED_EVENT, REOPENED_EVENT,
    READY_FOR_REVIEW_EVENT, PULL_REQUEST_REVIEW], last: 30) {
    nodes {
      __typename
      ... on PullRequestCommit { commit { committedDate } }
      ... on HeadRefForcePushedEvent { createdAt actor { login } }
      ... on ReopenedEvent { createdAt actor { login } }
      ... on ReadyForReviewEvent { createdAt actor { login } }
      ... on PullRequestReview { submittedAt author { login __typename } }
    }
  }"""
FIELDS = """
  number state isDraft mergeable reviewDecision baseRefName headRefOid headRefName createdAt authorAssociation
  author { login }
  headRepository { nameWithOwner }
  reviewRequests(first: 50) { nodes { requestedReviewer { ... on User { login } } } }
  commits(last: 1) { nodes { commit { pushedDate checkSuites(first: 20) { nodes { createdAt updatedAt } } } } }
  reviewThreads(last: 50) { nodes { comments(last: 5) { nodes { publishedAt author { login __typename } } } } }
  mergeQueueEntry { id }
  labels(first: 100) { nodes { name } }
  latestOpinionatedReviews(first: 50, writersOnly: true) {
    nodes { state submittedAt commit { oid } author { login __typename } }
  }
  %s
  comments(last: %d) { pageInfo { hasPreviousPage } nodes { createdAt author { login __typename } %s } }"""
LIST = """query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: %d, after: $cursor, orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes { %s }
    }
  }
}""" % (PAGE, FIELDS % (ACTIVITY, 30, ""))
# One pull request with its comments' text (the marker) and who added the label.
ONE = """query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) { %s
    labelEvents: timelineItems(itemTypes: [LABELED_EVENT], last: 100) {
      nodes { ... on LabeledEvent { createdAt actor { login } label { name } } }
    }
  } }
}""" % (FIELDS % (ACTIVITY, 100, "body"))


def when(stamp: str) -> datetime:
    return datetime.fromisoformat(stamp.replace("Z", "+00:00"))


def is_bot(node: dict) -> bool:
    return ((node.get("author") or {}).get("__typename")) == "Bot"


def pushed_at(pr: dict, *, reruns: bool = True) -> list[str]:
    """When the head arrived: `pushedDate` and check-suite timestamps (`updatedAt` updates even when GitHub reuses a
    suite from another branch for an older SHA). A commit's own date can be days before a push. `reruns=False`
    leaves out `updatedAt`, which a CI rerun or a reused suite also moves without the author doing anything."""
    head = (pr["commits"]["nodes"] or [{}])[-1].get("commit") or {}
    out = [head["pushedDate"]] if head.get("pushedDate") else []
    for suite in (head.get("checkSuites") or {}).get("nodes", []):
        for key in ("createdAt", "updatedAt") if reruns else ("createdAt",):
            if suite.get(key):
                out.append(suite[key])
    return out


def login_of(node: dict) -> str:
    return (((node.get("author") or {}).get("login")) or "").lower()


def author_activity(pr: dict) -> datetime:
    """The newest thing the pull request's author did: opened it, pushed a commit, reopened it, marked it ready for
    review, commented, reviewed, or replied in a review thread. A maintainer's comment or a CI rerun does not reset
    this clock: otherwise a PR that conflicts after the author has gone quiet is never swept (#19327)."""
    author = login_of(pr)
    times = [pr["createdAt"], *pushed_at(pr, reruns=False)]
    for thread in pr["reviewThreads"]["nodes"]:
        times += [c["publishedAt"] for c in thread["comments"]["nodes"]
                  if not is_bot(c) and login_of(c) == author]
    for item in pr["timelineItems"]["nodes"]:
        kind = item.get("__typename")
        if kind == "PullRequestReview":
            if not is_bot(item) and login_of(item) == author:
                times.append(item.get("submittedAt"))
        elif kind == "PullRequestCommit":
            times.append((item.get("commit") or {}).get("committedDate"))
        elif (((item.get("actor") or {}).get("login")) or "").lower() == author:
            # Reopened, ready for review, force-pushed: a maintainer doing it does not reset the author's clock.
            times.append(item.get("createdAt"))
    times += [c["createdAt"] for c in pr["comments"]["nodes"]
              if not is_bot(c) and login_of(c) == author]
    return max(when(t) for t in times if t)


def other_activity(pr: dict) -> datetime | None:
    """The newest comment, review or review-thread reply by someone other than the author and the bots, if any."""
    author = login_of(pr)
    times = []
    for thread in pr["reviewThreads"]["nodes"]:
        times += [c["publishedAt"] for c in thread["comments"]["nodes"]
                  if not is_bot(c) and login_of(c) != author]
    for item in pr["timelineItems"]["nodes"]:
        if item.get("__typename") == "PullRequestReview" and not is_bot(item) and login_of(item) != author:
            times.append(item.get("submittedAt"))
    times += [c["createdAt"] for c in pr["comments"]["nodes"]
              if not is_bot(c) and login_of(c) != author]
    return max((when(t) for t in times if t), default=None)


def last_activity(pr: dict) -> datetime:
    """The newest of: opened, a commit, the head's push, a push over the branch, a reopen, ready for review, a
    person's comment, review or reply in a review thread."""
    times = [pr["createdAt"], *pushed_at(pr)]
    for thread in pr["reviewThreads"]["nodes"]:
        times += [c["publishedAt"] for c in thread["comments"]["nodes"] if not is_bot(c)]
    for item in pr["timelineItems"]["nodes"]:
        if item.get("__typename") == "PullRequestReview" and is_bot(item):
            continue
        times.append(((item.get("commit") or {}).get("committedDate")) or item.get("createdAt")
                     or item.get("submittedAt"))
    times += [c["createdAt"] for c in pr["comments"]["nodes"] if not is_bot(c)]
    return max(when(t) for t in times if t)


def changes_review(pr: dict) -> dict | None:
    """The newest standing review that requested changes, if the head is still the commit it looked at."""
    if pr.get("reviewDecision") != "CHANGES_REQUESTED":
        return None
    asks = [r for r in pr["latestOpinionatedReviews"]["nodes"] if r["state"] == "CHANGES_REQUESTED"]
    if not asks:
        return None
    newest = max(asks, key=lambda r: r["submittedAt"])
    if (newest.get("commit") or {}).get("oid") != pr["headRefOid"]:
        return None
    # Asked to look again: answered without a push, now waiting on the reviewer.
    requested = {((r.get("requestedReviewer") or {}).get("login") or "").lower() for r in pr["reviewRequests"]["nodes"]}
    return None if ((newest.get("author") or {}).get("login") or "").lower() in requested else newest


def reasons(pr: dict, now: datetime, days: dict) -> set[str] | None:
    """The reasons it waits on its author now, or None when GitHub hasn't worked out whether it conflicts."""
    if pr["mergeable"] == "UNKNOWN":
        return None
    out = set()
    # The conflict clock is the author's: a maintainer's comment does not reset it. A pull request the author acted
    # on last is waiting on the team, not stale, so it is not swept either (#19327).
    authored = author_activity(pr)
    other = other_activity(pr)
    idle = now - authored
    if (pr["mergeable"] == "CONFLICTING" and idle >= timedelta(days=days[CONFLICT])
            and (other is None or authored <= other)):
        out.add(CONFLICT)
    review = changes_review(pr)
    if review and now - max(when(review["submittedAt"]), last_activity(pr)) >= timedelta(days=days[CHANGES]):
        out.add(CHANGES)
    return out


def excluded(pr: dict) -> str | None:
    labels = {l["name"].lower() for l in pr["labels"]["nodes"]}
    if pr["isDraft"]:
        return "draft"
    if pr["baseRefName"] != BASE:
        return "base"
    if pr.get("mergeQueueEntry"):
        return "queue"
    if labels & PROTECTED:
        return "protected"
    return None


def own_markers(pr: dict) -> list[tuple[datetime, set[str], str]]:
    """Every trusted stale-prs marker comment by this workflow's token, oldest to newest."""
    found = []
    for c in pr["comments"]["nodes"]:
        m = MARKER.search(c.get("body") or "")
        if m and is_bot(c) and (c.get("author") or {}).get("login") == BOT:
            found.append((when(c["createdAt"]), set(m.group(1).split(",")), m.group(2)))
    return found


def has_active_notice_without_label(pr: dict) -> bool:
    """True when a prior run already posted a notice for this exact head after `last_activity` (e.g. comment
    succeeded, then adding the `stale` label failed), so we only add the missing label and never post a duplicate."""
    markers = own_markers(pr)
    if not markers:
        return False
    at, _why, head = markers[-1]
    return head == pr["headRefOid"] and at >= last_activity(pr)


def notice_of(pr: dict) -> tuple[datetime, set[str], str] | None:
    """(when, reasons, head) of the notice that goes with the label now on: the workflow's token added the label
    last, and its marker comment was posted with that label (just before or after it; an older cycle's marker never
    counts). Returns None when a person or another workflow (such as `/label stale`) added the label without a
    matching marker, or when the comment window is incomplete."""
    added = [e for e in pr.get("labelEvents", {}).get("nodes", [])
             if ((e.get("label") or {}).get("name") or "").lower() == LABEL]
    if not added or (added[-1].get("actor") or {}).get("login") != BOT:
        return None
    label_at = when(added[-1]["createdAt"])
    # Notice is posted right before labelling (allow a small clock/API window, never an earlier cycle days ago).
    earliest = label_at - timedelta(minutes=5)
    for at, why, head in reversed(own_markers(pr)):
        if at >= earliest:
            return (at, why, head)
        return None
    # No marker in the last 100 comments (either `/label stale` added it, or the marker scrolled out): leave alone.
    return None


def labelled(pr: dict) -> bool:
    return LABEL in {l["name"].lower() for l in pr["labels"]["nodes"]}


def decide(pr: dict, now: datetime, days: dict) -> str:
    """skip, warn, clear (take the label off), wait, close or unclear (labelled without a matching workflow notice,
    e.g. by a person or `/label stale`, left alone). A labelled pull request needs its comments' text: the list
    leaves it out, Api.pull reads it."""
    if excluded(pr):
        return "clear" if labelled(pr) else "skip"
    now_reasons = reasons(pr, now, days)
    if not labelled(pr):
        return "warn" if now_reasons else "skip"
    notice = notice_of(pr)
    if notice is None:
        return "unclear"
    at, named, head = notice
    if pr["headRefOid"] != head or last_activity(pr) > at:
        return "clear"
    if now_reasons is None:
        return "wait"
    if not (named & now_reasons):
        return "clear"
    return "close" if now - at >= timedelta(days=days["grace"]) else "wait"


def notice_text(repo: str, why: set[str], head: str, days: dict) -> str:
    lines = []
    if CONFLICT in why:
        lines.append(f"- It conflicts with `{BASE}` and nothing has happened here for {days[CONFLICT]} days. "
                     f"Merge `{BASE}` into your branch.")
    if CHANGES in why:
        lines.append(f"- Changes were requested {days[CHANGES]} or more days ago and no commit followed. "
                     "Push the changes, or answer the review.")
    return ("This pull request is waiting on its author:\n" + "\n".join(lines) + "\n\n"
            f"It will be closed in {days['grace']} days unless something happens: any push or comment here takes "
            f"the `{LABEL}` label off. Away for a while? Ask a maintainer for `keep-open`. "
            f"[Why]({GUIDE.format(repo=repo)})\n\n"
            f"<!-- stale-prs: reasons={','.join(sorted(why))} head={head} -->")


def closing_text(repo: str, days: dict) -> str:
    return (f"Closing: nothing changed in the {days['grace']} days after the notice above. Thank you for the work! "
            "Once it's updated (merged with main, or the review answered), reopen it, or ask here and we will. "
            "Merge `main` rather than force-pushing: GitHub cannot reopen a pull request whose branch was "
            f"force-pushed after it closed. [Why]({GUIDE.format(repo=repo)})")


# --- GitHub ------------------------------------------------------------------------------------------------------


def gh(*args: str) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:2])}: {r.stderr.strip()[:300]}")
    return r.stdout.strip()


class Api:
    def __init__(self, repo: str, apply: bool):
        self.repo = repo
        self.apply = apply
        self.owner, self.name = repo.split("/")

    def _read(self, *args: str) -> str:
        # GitHub answers a heavy list now and then with a 5xx. Only reads repeat: a repeated comment is a second one.
        for attempt in range(1, READ_ATTEMPTS + 1):
            try:
                return gh(*args)
            except RuntimeError as e:
                if attempt == READ_ATTEMPTS or not any(f"HTTP {c}" in str(e) for c in (500, 502, 503, 504)):
                    raise
                time.sleep(attempt * READ_RETRY_S)
        raise AssertionError

    def _graphql(self, query: str, **variables) -> dict:
        args = ["api", "graphql", "-f", f"query={query}", "-f", f"owner={self.owner}", "-f", f"name={self.name}"]
        for key, value in variables.items():
            args += ["-F" if isinstance(value, int) else "-f", f"{key}={value}"]
        return json.loads(self._read(*args))["data"]["repository"]

    def open_pulls(self) -> list[dict]:
        out, cursor = [], None
        while True:
            page = self._graphql(LIST, **({"cursor": cursor} if cursor else {}))["pullRequests"]
            out += page["nodes"]
            if not page["pageInfo"]["hasNextPage"]:
                return out
            cursor = page["pageInfo"]["endCursor"]

    def pull(self, n: int) -> dict:
        return self._graphql(ONE, number=n)["pullRequest"]

    def labels(self) -> set[str]:
        return {n.lower() for n in gh("label", "list", "-R", self.repo, "--limit", "1000", "--json", "name",
                                      "--jq", ".[].name").splitlines()}

    def _write(self, *args: str) -> None:
        # Every write goes through here: a dry run never reaches GitHub, whatever token it holds.
        if not self.apply:
            raise AssertionError(f"dry run tried to write: gh {' '.join(args[:3])}")
        gh(*args)

    def label(self, n: int) -> None:
        self._write("api", f"repos/{self.repo}/issues/{n}/labels", "-f", f"labels[]={LABEL}")

    def unlabel(self, n: int) -> None:
        try:
            self._write("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/labels/{LABEL}")
        except RuntimeError as e:
            if "HTTP 404" not in str(e):  # already gone
                raise

    def comment(self, n: int, text: str) -> None:
        self._write("api", f"repos/{self.repo}/issues/{n}/comments", "-f", f"body={text}")

    def close(self, n: int) -> None:
        self._write("api", "-X", "PATCH", f"repos/{self.repo}/pulls/{n}", "-f", "state=closed")

    def create_label(self, name: str, color: str, description: str) -> None:
        self._write("label", "create", name, "-R", self.repo, "--color", color, "--description", description)


# Clears free a pull request that moved, so they go first; closes before warnings so a capped run still drains.
ORDER = {"clear": 0, "close": 1, "warn": 2}


def sweep(api, days: dict, max_actions: int, now: datetime | None = None, wait=time.sleep, log=print,
          closed: list | None = None) -> dict[str, list[int]]:
    """Every open pull request once; returns pull request numbers by outcome ('<action> next run' when capped).
    `closed` collects each closed pull request's head branch and repository, whose runs the workflow cancels."""
    if api.apply and LABEL not in api.labels():
        # Labelling would fail after the notice: the same pull request would get a notice every day.
        raise SystemExit(f"no `{LABEL}` label: run `stale_prs.py create --apply` first")
    prs = api.open_pulls()
    if any(p["mergeable"] == "UNKNOWN" and not excluded(p) for p in prs):
        # Asking started GitHub's computation; ask once more. What is still unknown waits for the next run.
        wait(MERGEABLE_WAIT_S)
        prs = api.open_pulls()
    now = now or datetime.now(timezone.utc)
    done: dict[str, list[int]] = {}
    todo = []
    for pr in prs:
        if labelled(pr) and not excluded(pr):
            pr = api.pull(pr["number"])  # the notice's marker is in the text, which the list leaves out
        action = decide(pr, now, days)
        if action in ORDER:
            todo.append((ORDER[action], pr["number"], action, pr))
        else:
            done.setdefault(action, []).append(pr["number"])
    acted = 0
    for _, n, action, pr in sorted(todo, key=lambda t: t[:2]):
        if acted >= max_actions:
            done.setdefault(f"{action} next run", []).append(n)
            continue
        if action != "clear":
            # The list is minutes old: a push, a comment, a draft, a label or a merge to main may have changed it.
            planned, pr = action, api.pull(n)
            if pr["state"] != "OPEN" or (action := decide(pr, now, days)) != planned:
                done.setdefault("recheck", []).append(n)
                continue
            if (
                action == "warn"
                and not has_active_notice_without_label(pr)
                and (pr.get("comments") or {}).get("pageInfo", {}).get("hasPreviousPage")
            ):
                # Comment window truncated without seeing our marker: skip rather than risk a duplicate notice.
                done.setdefault("recheck", []).append(n)
                continue
        done.setdefault(action, []).append(n)
        acted += 1
        why = sorted(reasons(pr, now, days) or ())
        log(f"#{n} {action}{' (' + ', '.join(why) + ')' if why and action != 'clear' else ''}"
            f"{'' if api.apply else ' (dry run)'}")
        if not api.apply:
            continue
        if action == "warn":
            # Comment first, then label: if labelling fails, the next run sees the marker for this head and only adds
            # the label (never posting a duplicate), and a `/label stale` without our marker stays `unclear` instead
            # of starting an auto-close cycle.
            if not has_active_notice_without_label(pr):
                api.comment(n, notice_text(api.repo, set(why), pr["headRefOid"], days))
            api.label(n)
        elif action == "clear":
            api.unlabel(n)
        elif action == "close":
            # Closed first: a failure between the two leaves a closed pull request without the note, never a second
            # note tomorrow. The label stays as the record; a reopen clears it.
            api.close(n)
            if closed is not None and pr.get("headRepository"):
                # A close by the workflow's token starts no workflow, so cancel-on-close.yml doesn't run for it.
                closed.append({
                    "number": n,
                    "branch": pr["headRefName"],
                    "repo": pr["headRepository"]["nameWithOwner"],
                    "head": pr["headRefOid"],
                    "closed_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
                })
            api.comment(n, closing_text(api.repo, days))
        wait(WRITE_PAUSE_S)
    unknown = sum(1 for p in prs if p["mergeable"] == "UNKNOWN" and not excluded(p))
    summary = ", ".join(f"{len(v)} {k}" for k, v in sorted(done.items()) if k != "skip") or "nothing to do"
    log(f"{'' if api.apply else 'Dry run. '}{len(prs)} open pull requests read, {unknown} still unknown: {summary}.")
    return done


LABELS = [
    (LABEL, "cfd3d7", "Waiting on its author: closed a few days after the notice unless updated"),
    ("keep-open", "0e8a16", "Never closed as stale"),
]


def create(api) -> list[str]:
    have = api.labels()
    todo = [label for label in LABELS if label[0] not in have]
    for label in todo if api.apply else []:
        api.create_label(*label)
    return [t[0] for t in todo]


def main(argv: list[str]) -> int:
    if argv[:1] == ["create"]:
        if argv[1:] not in ([], ["--apply"]):
            sys.exit(__doc__)
        for name in create(Api(os.environ.get("REPO", "horizon-rust/horizon"), argv[1:] == ["--apply"])):
            print(name)
        return 0
    p = argparse.ArgumentParser(usage=__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--dry-run", action="store_true")
    p.add_argument("--conflict-days", type=int, default=4)
    p.add_argument("--changes-days", type=int, default=7)
    p.add_argument("--grace-days", type=int, default=3)
    p.add_argument("--max-actions", type=int, default=30)
    a = p.parse_args(argv)
    days = {CONFLICT: a.conflict_days, CHANGES: a.changes_days, "grace": a.grace_days}
    closed: list = []
    try:
        sweep(Api(os.environ.get("REPO", "horizon-rust/horizon"), a.apply), days, a.max_actions, closed=closed)
    finally:
        # Even after a failed write: the pull requests closed before it still need their runs cancelled.
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
                out.write(f"closed={json.dumps(closed)}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
