#!/usr/bin/env python3
"""Anyone claims an issue before working on it (CONTRIBUTING.md "Claim before you start").

    claim_command.py command               one comment (claim-command.yml): assigns or unassigns, answers in the thread
    claim_command.py release [--dry-run]   the daily run: releases the claims older than 5 days with no open pull request

A comment on an issue, on its first line:

    /claim             assigns the commenter (GitHub lets an issue's commenters be assigned, no role needed)
    /unclaim           unassigns the commenter
    /unclaim @login    the repository owner, the council and trusted reviewers release someone else's claim

Trailing command punctuation is allowed. `/claim` ignores explanatory text; `/unclaim` targets the commenter
unless the entire remainder of that first line is a login.

An assigned issue is taken, and so is one claimed on Discord (Kira's `in progress` label). Trackers and proposals
(labels `tracker`, `proposal`, `not claimable`, set by maintainers) are never claimed whole. Refused with a one-line
answer: a pull request (claim the issue it fixes), a closed or locked issue, an issue someone else holds, or one an
open pull request by someone else already closes (`Fixes`, `Closes` or `Resolves #N`, or the sidebar's link).
A success answers with a 👍 on the comment; the assignment shows in the thread, and the issue's `help wanted` label
becomes `claimed` (and back when its last holder unclaims).
A contributor holds at most 5 claimed issues without an open pull request of theirs that links one: a `/claim`
past that is refused with the count (owner, 2026-11-10). Claims already over the limit stay. The repository's owner,
members and collaborators (the comment's author association), the trusted reviewers and the logins in the repository
variable `CLAIM_LIMIT_EXEMPT` (comma-separated) are not limited.
A claim made here is released after 5 days unless an open pull request links the issue (`Fixes #N`, a mention, or the
sidebar's link); assignments made by hand are never released. No code of any pull request is read or run: only the
comment's text, its author and the API's answers, which reach this script as env vars.
"""
from __future__ import annotations

import json
import os
import re
import string
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import nightly_set as ns  # noqa: E402  (the same trusted reviewers and owner)

PREFIX = "🤖 Agent: "
STALE = timedelta(days=5)
LOGIN = re.compile(r"^@?([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))$")
# The login GitHub shows for this workflow's token: `github-actions[bot]` in REST, `github-actions` in GraphQL.
BOT_LOGINS = ("github-actions", "github-actions[bot]")
# Kira's Discord `/claim` marks an issue with this label (horizon-rust/kira, WORK.label): it is taken too.
KIRA_LABEL = "in progress"
# Issues nobody claims whole (ari, 2026-11-02): a tracker lists other issues, a proposal waits for a decision.
UNCLAIMABLE = ("tracker", "proposal", "not claimable")
# Claimed issues a contributor may hold with no open pull request of theirs linking them (owner, 2026-11-10).
LIMIT = 5
# Author associations GitHub gives the repository's own people: they are not limited.
STAFF = ("OWNER", "MEMBER", "COLLABORATOR")


class Refused(Exception):
    """A command that changes nothing; its text is the answer."""


@dataclass
class Command:
    verb: str  # claim, unclaim
    who: str | None = None  # unclaim @login


def safe(text: str) -> str:
    """Text of a comment, fit to quote back: no markdown, no mentions."""
    return re.sub(r"[^\w#:. -]", "", text)[:60].strip() or "?"


def parse(body: str) -> Command | None:
    """The first line that starts with a command; its first word may carry trailing punctuation, and only an exact
    unclaim login selects another person."""
    for line in body.splitlines():
        # Agents start every comment with "🤖 Agent:" (CLAUDE.md): their `/claim` follows it, on its line or the next.
        words = re.sub(r"^\s*🤖\s*Agent:", "", line).split(maxsplit=1)
        command = words[0].rstrip(string.punctuation).lower() if words else ""
        if command in ("/claim", "/unclaim"):
            verb = command[1:]
            rest = words[1] if len(words) > 1 else ""
            who = LOGIN.match(rest) if verb == "unclaim" else None
            return Command(verb, who.group(1) if who else None)
    return None


def is_team(actor: ns.Actor, reviewers: ns.Reviewers, repo: str) -> bool:
    """The repository owner or an account trusted by numeric ID."""
    return (actor.login.lower() == repo.split("/")[0].lower() or ns.is_owner(actor)
            or (reviewers.listed(actor.id) and not reviewers.paused))


def assignees(issue: dict) -> list[str]:
    return [a["login"] for a in issue.get("assignees") or []]


def target_refusal(issue: dict) -> str | None:
    """Why this thread can't be claimed or released (None when it can). `issue` is GitHub's issue object."""
    if "pull_request" in issue:
        return "Pull requests aren't claimed: claim the issue this one fixes, or open one first."
    if issue.get("locked"):
        return "This issue is locked."
    if issue.get("state") == "closed":
        return "This issue is closed."
    return None


# --- GitHub ------------------------------------------------------------------------------------------------------


def gh(*args: str, check: bool = True, stdin: str | None = None) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, input=stdin)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {r.stderr.strip()[:300]}")
    return r.stdout.strip() if r.returncode == 0 else ""


LINKS = """
fragment Links on Issue {
  closedByPullRequestsReferences(first: 50, includeClosedPrs: false) {
    pageInfo { hasNextPage endCursor } nodes { state author { login } }
  }
  timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT]) {
    pageInfo { hasNextPage endCursor }
    nodes { ... on CrossReferencedEvent { source { ... on PullRequest { state author { login } } } } }
  }
}"""

HELD_ISSUES = """
query($owner: String!, $name: String!, $login: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, states: OPEN, filterBy: {assignee: $login}, after: $cursor) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        ...Links
      }
    }
  }
}""" + LINKS

# The rest of one issue's links, when HELD_ISSUES got only their first page; each connection is paged on its own.
ISSUE_LINKS = """
query($owner: String!, $name: String!, $number: Int!, $refs: String, $events: String, $wantRefs: Boolean!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      closedByPullRequestsReferences(first: 50, after: $refs, includeClosedPrs: false) @include(if: $wantRefs) {
        pageInfo { hasNextPage endCursor } nodes { state author { login } }
      }
      timelineItems(first: 100, after: $events, itemTypes: [CROSS_REFERENCED_EVENT]) @skip(if: $wantRefs) {
        pageInfo { hasNextPage endCursor }
        nodes { ... on CrossReferencedEvent { source { ... on PullRequest { state author { login } } } } }
      }
    }
  }
}"""

# The open pull requests that would close one issue: someone's work already covers it.
COVERING = """
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      closedByPullRequestsReferences(first: 20, includeClosedPrs: false) { nodes { number state author { login } } }
    }
  }
}"""

ASSIGNED_ISSUES = """
query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, states: OPEN, filterBy: {assignee: "*"}, after: $cursor) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        assignees(first: 10) { nodes { login } }
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { number } }
        timelineItems(last: 100, itemTypes: [ASSIGNED_EVENT, CROSS_REFERENCED_EVENT]) {
          nodes {
            __typename
            ... on AssignedEvent { createdAt actor { login } assignee { ... on User { login } } }
            ... on CrossReferencedEvent { source { ... on PullRequest { number state } } }
          }
        }
      }
    }
  }
}"""


class Api:
    """The few GitHub calls this needs; tests replace it."""

    def __init__(self, repo: str):
        self.repo = repo

    def issue(self, n: int) -> dict | None:
        raw = gh("api", f"repos/{self.repo}/issues/{n}", check=False)
        return json.loads(raw) if raw else None

    def assign(self, n: int, login: str) -> list[str]:
        """Assigns `login`; returns the assignees GitHub kept (it silently drops someone it can't assign)."""
        raw = gh("api", f"repos/{self.repo}/issues/{n}/assignees", "-f", f"assignees[]={login}")
        return assignees(json.loads(raw))

    def unassign(self, n: int, login: str) -> None:
        gh("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/assignees", "-f", f"assignees[]={login}")

    def label(self, n: int, add: str, remove: str) -> None:
        """Swaps one label for another; removing a label the issue lacks is not an error."""
        gh("api", f"repos/{self.repo}/issues/{n}/labels", "-f", f"labels[]={add}")
        gh("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/labels/{remove.replace(' ', '%20')}", check=False)

    def graphql(self, query: str, **variables) -> dict:
        owner, name = self.repo.split("/")
        args = ["api", "graphql", "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={query}"]
        for key, value in variables.items():
            if value is not None:
                args += ["-F", f"{key}={str(value).lower() if isinstance(value, bool) else value}"]
        return json.loads(gh(*args))["data"]["repository"]

    def held_issues(self, login: str) -> list[dict]:
        """The open issues assigned to `login`, each with every open pull request that links it (all pages)."""
        out, cursor = [], None
        while True:
            page = self.graphql(HELD_ISSUES, login=login, cursor=cursor)["issues"]
            out += page["nodes"]
            if not page["pageInfo"]["hasNextPage"]:
                break
            cursor = page["pageInfo"]["endCursor"]
        for issue in out:
            for field, cursor_name in (("closedByPullRequestsReferences", "refs"), ("timelineItems", "events")):
                links = issue[field]
                while links["pageInfo"]["hasNextPage"]:
                    more = self.graphql(ISSUE_LINKS, number=issue["number"], wantRefs=cursor_name == "refs",
                                        **{cursor_name: links["pageInfo"]["endCursor"]})["issue"][field]
                    links["nodes"] += more["nodes"]
                    links["pageInfo"] = more["pageInfo"]
        return out

    def covering_pull_requests(self, n: int) -> list[dict]:
        """The pull requests whose closing references name issue `n` (open ones, though check `state`)."""
        return self.graphql(COVERING, number=n)["issue"]["closedByPullRequestsReferences"]["nodes"]

    def react(self, comment_id: int) -> None:
        gh("api", f"repos/{self.repo}/issues/comments/{comment_id}/reactions", "-f", "content=+1")

    def comment(self, n: int, text: str) -> None:
        gh("api", f"repos/{self.repo}/issues/{n}/comments", "-f", f"body={text}")

    def assigned_issues(self) -> list[dict]:
        owner, name = self.repo.split("/")
        out, cursor = [], None
        while True:
            args = ["api", "graphql", "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={ASSIGNED_ISSUES}"]
            if cursor:
                args += ["-F", f"cursor={cursor}"]
            page = json.loads(gh(*args))["data"]["repository"]["issues"]
            out += page["nodes"]
            if not page["pageInfo"]["hasNextPage"]:
                return out
            cursor = page["pageInfo"]["endCursor"]


def exempt_logins(raw: str) -> set[str]:
    """The logins of `CLAIM_LIMIT_EXEMPT`, comma-separated, compared without case as GitHub does."""
    return {login.strip().lstrip("@").lower() for login in raw.split(",") if login.strip().lstrip("@")}


def without_pull_request(issues: list[dict], login: str) -> list[int]:
    """The numbers of `issues` (GraphQL nodes from HELD_ISSUES) that no open pull request by `login` links."""
    def by_them(pr: dict | None) -> bool:
        pr = pr or {}
        return pr.get("state") == "OPEN" and ((pr.get("author") or {}).get("login") or "").lower() == login.lower()
    return [i["number"] for i in issues
            if not any(by_them(pr) for pr in i["closedByPullRequestsReferences"]["nodes"])
            and not any(by_them(e.get("source")) for e in i["timelineItems"]["nodes"])]


def run(api, cmd: Command, actor: ns.Actor, number: int, reviewers: ns.Reviewers,
        association: str = "NONE", exempt: frozenset[str] | set[str] = frozenset()) -> str | None:
    """Does the command; returns the comment to post, or None when a 👍 says it. Raises Refused for a refusal.
    Claims are GitHub assignees, so they are held by login; who may act for others is decided by ID.
    `association` is the comment's author association, which exempts the repository's own people from the limit, and
    `exempt` the lowercased logins of `CLAIM_LIMIT_EXEMPT`."""
    login = actor.login
    issue = api.issue(number)
    if issue is None:
        raise Refused("There is no such issue.")
    if (why := target_refusal(issue)):
        raise Refused(why)
    holders = assignees(issue)
    held = {h.lower() for h in holders}
    if cmd.verb == "claim":
        if login.lower() in held:
            # A claim made before the label existed, or by hand, gets it now.
            if "claimed" not in {str(l.get("name", "")).lower() for l in issue.get("labels") or []}:
                api.label(number, "claimed", "help wanted")
            raise Refused("You already hold this issue.")
        if holders:
            names = ", ".join(f"@{h}" for h in holders)
            raise Refused(f"{names} already claimed this issue: pick another, or ask them here.")
        labels = {str(l.get("name", "")).lower() for l in issue.get("labels") or []}
        if (kind := next((u for u in UNCLAIMABLE if u in labels), None)):
            raise Refused(f"this issue is labelled `{kind}`, so it isn't claimed as a whole: claim one of the issues "
                          "it lists, or open one for the part you take.")
        if KIRA_LABEL in labels:
            raise Refused(f"someone claimed this issue on Discord (Kira's `{KIRA_LABEL}` label): pick another, or ask "
                          "in the thread.")
        covering = sorted(pr["number"] for pr in api.covering_pull_requests(number) if pr.get("state") == "OPEN"
                          and ((pr.get("author") or {}).get("login") or "").lower() != login.lower())
        if covering:
            listed = ", ".join(f"#{n}" for n in covering)
            raise Refused(f"this issue is covered by {listed}, an open pull request that closes it: pick another, or "
                          "ask its author in that pull request.")
        limited = association not in STAFF and login.lower() not in exempt and not is_team(actor, reviewers, api.repo)
        if limited:
            open_claims = without_pull_request(api.held_issues(login), login)
            if len(open_claims) >= LIMIT:
                listed = ", ".join(f"#{n}" for n in sorted(open_claims))
                raise Refused(f"you already hold {len(open_claims)} claimed issues with no open pull request of yours "
                              f"({listed}), and the limit is {LIMIT}. Opening a pull request that links one of them, "
                              "or `/unclaim` on one, frees a slot.")
        if login.lower() not in {a.lower() for a in api.assign(number, login)}:
            raise Refused("GitHub didn't assign you; nothing changed.")
        # Runs for different issues aren't serialized (one queue for the whole repository would let GitHub cancel
        # the pending runs of a burst, leaving comments unanswered), so two claims made at once can both pass the
        # count above. Counting again after assigning catches that: whoever then sees more than the limit backs out.
        if limited:
            try:
                recount = len(without_pull_request(api.held_issues(login), login))
            except RuntimeError:  # an unchecked claim isn't kept: undo it, then the caller reports GitHub's refusal
                api.unassign(number, login)
                raise
        if limited and recount > LIMIT:
            api.unassign(number, login)
            raise Refused(f"another of your claims was made at the same moment and together they pass the limit of "
                          f"{LIMIT} claimed issues with no open pull request of yours, so this one was undone. "
                          "`/claim` again if you still have a free slot.")
        api.label(number, "claimed", "help wanted")
        return None
    who = cmd.who or login
    if who.lower() != login.lower() and not is_team(actor, reviewers, api.repo):
        raise Refused(f"Only the repository owner, the council and the trusted reviewers in `{ns.REVIEWERS}` "
                      "release someone else's claim.")
    if who.lower() not in held:
        raise Refused("You don't hold this issue." if who.lower() == login.lower() else f"`{safe(who)}` doesn't hold this issue.")
    real = next(h for h in holders if h.lower() == who.lower())
    api.unassign(number, real)
    if len(holders) == 1:
        api.label(number, "help wanted", "claimed")
    if who.lower() == login.lower():
        return None
    return f"{PREFIX}released @{real}'s claim, on behalf of @{login}. `/claim` takes it."


def command() -> int:
    body, login = os.environ["BODY"], os.environ["AUTHOR"]
    number, repo, comment_id = int(os.environ["ISSUE"]), os.environ["REPO"], int(os.environ["COMMENT_ID"])
    api: Api = Api(repo)
    try:
        cmd = parse(body)
        if cmd is None:
            return 0
        try:
            actor = ns.actor_from_env()
        except ns.Refused as e:
            raise Refused(str(e)) from e
        reviewers = ns.read_reviewers(Path(ns.REVIEWERS).read_text())
        text = run(api, cmd, actor, number, reviewers, os.environ.get("AUTHOR_ASSOCIATION", "NONE"),
                   exempt_logins(os.environ.get("CLAIM_LIMIT_EXEMPT", "")))
    except Refused as e:
        text = f"{PREFIX}refused, {e} (asked by @{safe(login)})"
    except RuntimeError as e:
        print(e, file=sys.stderr)
        text = f"{PREFIX}GitHub refused that, nothing more was done (asked by @{safe(login)})."
    if text is None:
        api.react(comment_id)
    else:
        api.comment(number, text)
    return 0


# --- The daily release --------------------------------------------------------------------------------------------


def stale_claims(issue: dict, now: datetime) -> list[str]:
    """The assignees of one issue (GraphQL node) whose claim this workflow made over 5 days ago, when no open pull
    request links the issue."""
    events = issue["timelineItems"]["nodes"]
    if issue["closedByPullRequestsReferences"]["nodes"] or any(
            e["__typename"] == "CrossReferencedEvent" and (e.get("source") or {}).get("state") == "OPEN"
            for e in events):
        return []
    last: dict[str, dict] = {}  # each assignee's newest assignment
    for e in events:
        if e["__typename"] == "AssignedEvent" and (e.get("assignee") or {}).get("login"):
            last[e["assignee"]["login"].lower()] = e
    out = []
    for node in issue["assignees"]["nodes"]:
        e = last.get(node["login"].lower())
        if not e or (e.get("actor") or {}).get("login") not in BOT_LOGINS:
            continue  # assigned by a person (or before the window GitHub returns): never released
        when = datetime.fromisoformat(e["createdAt"].replace("Z", "+00:00"))
        if now - when > STALE:
            out.append(node["login"])
    return out


def release(api, now: datetime, dry_run: bool) -> list[tuple[int, str]]:
    done = []
    for issue in api.assigned_issues():
        stale = stale_claims(issue, now)
        for login in stale:
            done.append((issue["number"], login))
            if dry_run:
                continue
            api.unassign(issue["number"], login)
            api.comment(issue["number"],
                        f"{PREFIX}released @{login}'s claim: {STALE.days} days without an open pull request that links "
                        f"this issue. `/claim` again when you pick it back up (a draft pull request with "
                        f"`Fixes #{issue['number']}` keeps the claim).")
        if stale and not dry_run and len(stale) == len(issue["assignees"]["nodes"]):
            api.label(issue["number"], "help wanted", "claimed")
    return done


if __name__ == "__main__":
    if sys.argv[1:] == ["command"]:
        sys.exit(command())
    if sys.argv[1:2] == ["release"] and sys.argv[2:] in ([], ["--dry-run"]):
        dry = sys.argv[2:] == ["--dry-run"]
        for n, who in release(Api(os.environ["REPO"]), datetime.now(timezone.utc), dry):
            print(f"{'would release' if dry else 'released'} #{n} @{who}")
        sys.exit(0)
    sys.exit(__doc__)
