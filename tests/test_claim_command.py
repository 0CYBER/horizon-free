import json
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest import mock

from tools.ci import claim_command as cc
from tools.ci import nightly_set as ns

ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 10, 29, 12, tzinfo=timezone.utc)
# Fixture accounts: the decision is by numeric ID, so each test login has one. Ludovic's real ID may not be filled in
# yet (tools/ci/resolve_reviewer_ids.py), so tests use their own unless it is.
LUDOVIC_ID = ns.OWNER_ID or 1
ns.OWNER_ID = LUDOVIC_ID
cc.ns.OWNER_ID = LUDOVIC_ID  # the script's own import of nightly_set
IDS = {"ludovic111": LUDOVIC_ID, "kacpherek": 101, "rdbtcvs": 102, "levzzz5154": 104}


def actor(login):
    return ns.Actor(login, IDS.get(login.lower(), 9999))


def listing(*lines):
    """A reviewers file: `kacpherek` becomes `101 kacpherek`; other lines stay as written."""
    return ns.read_reviewers("".join((f"{IDS[l.lower()]} {l}" if l.lower() in IDS else l) + "\n" for l in lines))


LIST = listing("kacpherek", "rdbtCVS")


def thread(state="open", holders=(), pr=False, locked=False, author="filer", age=timedelta(days=1), bot=False):
    t = {"state": state, "assignees": [{"login": h} for h in holders], "locked": locked,
         "user": {"login": author, "type": "Bot" if bot else "User"},
         "created_at": (NOW - age).strftime("%Y-%m-%dT%H:%M:%SZ")}
    if pr:
        t["pull_request"] = {"merged_at": None}
    return t


class Fake:
    def __init__(self, threads=None, claims=0, assignable=True):
        self.threads = {5: thread()} if threads is None else threads
        self.claims, self.assignable, self.calls = claims, assignable, []

    def issue(self, n):
        return self.threads.get(n)

    def assign(self, n, login):
        self.calls.append(("assign", n, login))
        return cc.assignees(self.threads[n]) + ([login] if self.assignable else [])

    def unassign(self, n, login):
        self.calls.append(("unassign", n, login))

    def open_claims(self, login):
        return self.claims

    def comment(self, n, text):
        self.calls.append(("comment", n, text))


def do(text, api=None, login="newcomer", reviewers=LIST, n=5):
    api = api or Fake()
    return cc.run(api, cc.parse(text), actor(login), n, reviewers, NOW), api


class ParseTests(unittest.TestCase):
    def test_commands(self):
        self.assertEqual(cc.parse("/claim"), cc.Command("claim"))
        self.assertEqual(cc.parse("  /CLAIM  \nI'll take it"), cc.Command("claim"))
        self.assertEqual(cc.parse("/unclaim"), cc.Command("unclaim"))
        self.assertEqual(cc.parse("/unclaim @Some-One"), cc.Command("unclaim", "Some-One"))
        self.assertEqual(cc.parse("/unclaim some-one"), cc.Command("unclaim", "some-one"))
        # An agent's comment starts with "🤖 Agent:" (CLAUDE.md); its command follows.
        self.assertEqual(cc.parse("🤖 Agent: /claim"), cc.Command("claim"))
        self.assertEqual(cc.parse("🤖 Agent: /unclaim @Some-One"), cc.Command("unclaim", "Some-One"))

    def test_not_commands(self):
        for text in ("", "claim", "/claimed", "/claims", "thanks\n/claim", "/close", "/unclaimed"):
            self.assertIsNone(cc.parse(text), text)

    def test_malformed(self):
        for text in ("/claim this one", "/claim @other", "/unclaim @a b", "/unclaim @-bad", "/unclaim <x>"):
            with self.assertRaises(cc.Refused, msg=text):
                cc.parse(text)


class ClaimTests(unittest.TestCase):
    def test_claim_a_free_issue(self):
        text, api = do("/claim")
        self.assertIsNone(text)  # the 👍 says it
        self.assertEqual(api.calls, [("assign", 5, "newcomer")])

    def test_taken(self):
        with self.assertRaisesRegex(cc.Refused, "@other already claimed"):
            do("/claim", Fake({5: thread(holders=("other",))}))

    def test_claimed_on_discord(self):
        t = thread()
        t["labels"] = [{"name": "In progress"}]
        api = Fake({5: t})
        with self.assertRaisesRegex(cc.Refused, "on Discord"):
            do("/claim", api)
        self.assertEqual(api.calls, [])

    def test_trackers_and_proposals_are_not_claimed(self):
        for name in ("Tracker", "proposal", "not claimable"):
            t = thread()
            t["labels"] = [{"name": "bug"}, {"name": name}]
            api = Fake({5: t})
            with self.assertRaisesRegex(cc.Refused, "isn't claimed as a whole"):
                do("/claim", api)
            self.assertEqual(api.calls, [], name)

    def test_already_yours(self):
        with self.assertRaisesRegex(cc.Refused, "already hold"):
            do("/claim", Fake({5: thread(holders=("NewComer",))}))

    def test_not_claimable(self):
        for t in (thread(pr=True), thread(state="closed"), thread(locked=True)):
            api = Fake({5: t})
            with self.assertRaises(cc.Refused):
                do("/claim", api)
            self.assertEqual(api.calls, [])
        with self.assertRaisesRegex(cc.Refused, "no such"):
            do("/claim", Fake({}))

    def test_five_claims_at_most(self):
        api = Fake(claims=cc.MAX_CLAIMS)
        with self.assertRaisesRegex(cc.Refused, "8 open issues"):
            do("/claim", api)
        self.assertEqual(api.calls, [])
        do("/claim", Fake(claims=cc.MAX_CLAIMS - 1))

    def test_only_the_claimants_own_pull_request_lifts_an_issue_off_the_limit(self):
        def closing(*authors, more=False):
            return {"pageInfo": {"hasNextPage": more, "endCursor": "k" if more else None},
                    "nodes": [{"author": {"login": a} if a else None} for a in authors]}

        def node(number, *authors, more=False):
            return {"number": number, "closedByPullRequestsReferences": closing(*authors, more=more)}

        # The claimant's pull request is the eleventh on issue 5: it is on the connection's second page.
        nodes = [node(1), node(2, "someone-else"), node(3, "Newcomer"), node(4, "someone-else", "newcomer"),
                 node(5, *["someone-else"] * 10, more=True), node(6, None)]
        pages = [{"pageInfo": {"hasNextPage": True, "endCursor": "c"}, "nodes": nodes[:2]},
                 {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes[2:]}]
        asked = []

        def gh(*args, **kwargs):
            asked.append(args)
            if any(a == "number=5" for a in args):
                return json.dumps({"data": {"repository": {"issue": {
                    "closedByPullRequestsReferences": closing("newcomer")}}}})
            return json.dumps({"data": {"repository": {"issues": pages[sum("login=newcomer" in a for a in asked) - 1]}}})

        with mock.patch.object(cc, "gh", gh):
            # Not linked, linked by another account, a deleted account: counted. Linked by the claimant, on the
            # first page or a later one: not.
            self.assertEqual(cc.Api("o/r").open_claims("newcomer"), 3)
        self.assertEqual(len(asked), 3)

    def test_team_has_no_limit_unless_paused(self):
        do("/claim", Fake(claims=50), login="ludovic111")
        do("/claim", Fake(claims=50), login="RdbtCVS")
        with self.assertRaises(cc.Refused):
            do("/claim", Fake(claims=50), login="rdbtCVS", reviewers=listing("rdbtCVS", "paused"))

    def test_github_dropped_the_assignee(self):
        with self.assertRaisesRegex(cc.Refused, "didn't assign"):
            do("/claim", Fake(assignable=False))


class GraceTests(unittest.TestCase):
    def test_non_author_is_refused_inside_the_window(self):
        api = Fake({5: thread(age=timedelta(minutes=2))})
        with self.assertRaises(cc.Refused):
            do("/claim", api)
        with self.assertRaises(cc.Refused):
            do("/claim", Fake({5: thread(age=timedelta(minutes=59))}), login="ludovic111")
        self.assertEqual(api.calls, [])

    def test_refusal_text(self):
        with self.assertRaises(cc.Refused) as e:
            do("/claim", Fake({5: thread(age=timedelta(minutes=2))}))
        self.assertEqual(str(e.exception), "@filer filed this issue 2 minutes ago: they have the first hour to claim it "
                                           "(until 12:58 UTC).")

    def test_author_claims_inside_the_window(self):
        text, api = do("/claim", Fake({5: thread(age=timedelta(minutes=2))}), login="Filer")
        self.assertIsNone(text)
        self.assertEqual(api.calls, [("assign", 5, "Filer")])

    def test_anyone_claims_after_the_window(self):
        do("/claim", Fake({5: thread(age=cc.AUTHOR_GRACE)}))

    def test_bot_filed_issues_have_no_window(self):
        do("/claim", Fake({5: thread(age=timedelta(seconds=5), bot=True)}))
        do("/claim", Fake({5: thread(age=timedelta(seconds=5), author="kira-horizon[bot]")}))


class UnclaimTests(unittest.TestCase):
    def test_own(self):
        text, api = do("/unclaim", Fake({5: thread(holders=("NewComer",))}))
        self.assertIsNone(text)
        self.assertEqual(api.calls, [("unassign", 5, "NewComer")])

    def test_not_held(self):
        with self.assertRaisesRegex(cc.Refused, "don't hold"):
            do("/unclaim", Fake({5: thread(holders=("other",))}))

    def test_someone_elses_needs_the_team(self):
        api = Fake({5: thread(holders=("Other",))})
        with self.assertRaisesRegex(cc.Refused, "trusted reviewers"):
            do("/unclaim @other", api)
        self.assertEqual(api.calls, [])
        text, api = do("/unclaim @other", Fake({5: thread(holders=("Other",))}), login="kacpherek")
        self.assertIn("released @Other's claim, on behalf of @kacpherek", text)
        self.assertEqual(api.calls, [("unassign", 5, "Other")])

    def test_someone_who_holds_nothing(self):
        with self.assertRaisesRegex(cc.Refused, "`ghost` doesn't hold"):
            do("/unclaim @ghost", login="ludovic111")


def node(number=7, holders=("dev",), events=(), closing=()):
    return {"number": number, "assignees": {"nodes": [{"login": h} for h in holders]},
            "closedByPullRequestsReferences": {"nodes": [{"number": c} for c in closing]},
            "timelineItems": {"nodes": list(events)}}


def assigned(login, days_ago, actor="github-actions"):
    when = (NOW - timedelta(days=days_ago)).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {"__typename": "AssignedEvent", "createdAt": when, "actor": {"login": actor}, "assignee": {"login": login}}


def mentioned(state):
    return {"__typename": "CrossReferencedEvent", "source": {"number": 9, "state": state}}


class ReleaseTests(unittest.TestCase):
    def test_stale_claim_released(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 6)]), NOW), ["dev"])

    def test_fresh_claim_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 4)]), NOW), [])

    def test_newest_assignment_counts(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), assigned("dev", 1)]), NOW), [])

    def test_hand_assignments_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 30, actor="kacpherek")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[]), NOW), [])

    def test_an_open_pull_request_keeps_it(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("OPEN")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9)], closing=(9,)), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("CLOSED")]), NOW), ["dev"])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), {"__typename": "CrossReferencedEvent",
                                                                          "source": {}}]), NOW), ["dev"])

    def test_a_saturated_timeline_window_keeps_the_claim(self):
        # With `last: 100`, an open PR link older than the newest 100 events is invisible. Keep the claim
        # rather than unassigning a contributor whose protection we cannot see.
        full = [assigned("other", 1) for _ in range(cc.TIMELINE_WINDOW)]
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 6), *full]), NOW), [])

    def test_release_unassigns_and_says_why(self):
        class Api(Fake):
            def assigned_issues(self):
                return [node(7, ("a", "b"), [assigned("a", 6), assigned("b", 2)]), node(8, ("c",), [assigned("c", 8)])]

        api = Api()
        self.assertEqual(cc.release(api, NOW, dry_run=True), [(7, "a"), (8, "c")])
        self.assertEqual(api.calls, [])
        cc.release(api, NOW, dry_run=False)
        self.assertEqual([c[:3] for c in api.calls if c[0] == "unassign"], [("unassign", 7, "a"), ("unassign", 8, "c")])
        comments = [c[2] for c in api.calls if c[0] == "comment"]
        self.assertEqual(len(comments), 2)
        self.assertTrue(comments[0].startswith(cc.PREFIX + "released @a's claim: 5 days"))


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/claim-command.yml").read_text()
        run_lines = [l.strip() for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["run: python3 tools/ci/claim_command.py command",
                                     "run: python3 tools/ci/claim_command.py release"])
        self.assertEqual(text.count("persist-credentials: false"), 2)
        self.assertEqual(text.count("ref: main"), 2)
        self.assertNotIn("pull_request_target", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull-requests: write", text)
        for line in text.splitlines():
            if "github.event.comment" in line and "run:" in line:
                self.fail(f"comment data in a shell line: {line}")


if __name__ == "__main__":
    unittest.main()
