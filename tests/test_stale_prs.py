import os
import tempfile
import unittest
from unittest import mock
from datetime import datetime, timedelta, timezone

from tools.ci import stale_prs as sp

NOW = datetime(2026, 11, 21, 12, 0, tzinfo=timezone.utc)
DAYS = {sp.CONFLICT: 4, sp.CHANGES: 7, "grace": 3}
HEAD = "a" * 40
OTHER = "b" * 40


def ago(days: float) -> str:
    return (NOW - timedelta(days=days)).strftime("%Y-%m-%dT%H:%M:%SZ")


def comment(at, login="someone", bot=False, body=""):
    return {"createdAt": at, "author": {"login": login, "__typename": "Bot" if bot else "User"}, "body": body}


def notice(at, reasons=(sp.CONFLICT,), head=HEAD, login=sp.BOT):
    return comment(at, login=login, bot=True, body=sp.notice_text("o/r", set(reasons), head, DAYS))


def pr(n=1, *, mergeable="CONFLICTING", idle=10, draft=False, base="main", queue=False, labels=(), review=None,
       comments=(), head=HEAD, state="OPEN", label_at=None, label_by=sp.BOT, pushed=None, suite_updated=None,
       requested=(), threads=(), has_prev_comments=False):
    """An open pull request whose last commit is `idle` days old, pushed then unless `pushed` days ago; `review` is
    (days ago, commit it looked at); `requested` the logins asked to review; `threads` review-thread replies. A
    `stale` label was added by `label_by` at `label_at`, by default a second after the newest marker."""
    reviews = []
    if review:
        reviews.append({"state": "CHANGES_REQUESTED", "submittedAt": ago(review[0]), "commit": {"oid": review[1]},
                        "author": {"login": "reviewer", "__typename": "User"}})
    suite = {"createdAt": ago(idle if pushed is None else pushed)}
    if suite_updated is not None:
        suite["updatedAt"] = ago(suite_updated)
    return {"number": n, "state": state, "isDraft": draft, "mergeable": mergeable, "baseRefName": base,
            "reviewDecision": "CHANGES_REQUESTED" if review else None, "headRefOid": head, "createdAt": ago(60),
            "authorAssociation": "CONTRIBUTOR", "author": {"login": "author"}, "headRefName": "topic",
            "headRepository": {"nameWithOwner": "fork/r"},
            "reviewRequests": {"nodes": [{"requestedReviewer": {"login": login}} for login in requested]},
            "commits": {"nodes": [{"commit": {"pushedDate": ago(idle if pushed is None else pushed),
                                              "checkSuites": {"nodes": [suite]}}}]},
            "reviewThreads": {"nodes": [{"comments": {"nodes": [reply]}} for reply in threads]},
            "mergeQueueEntry": {"id": "q"} if queue else None,
            "labels": {"nodes": [{"name": name} for name in labels]},
            "latestOpinionatedReviews": {"nodes": reviews},
            "timelineItems": {"nodes": [{"__typename": "PullRequestCommit", "commit": {"committedDate": ago(idle)}}]},
            "comments": {"pageInfo": {"hasPreviousPage": has_prev_comments}, "nodes": list(comments)},
            "labelEvents": {"nodes": [{"createdAt": label_at or label_time(comments), "actor": {"login": label_by},
                                       "label": {"name": "stale"}}] if "stale" in labels else []}}


def label_time(comments):
    markers = [c["createdAt"] for c in comments if sp.MARKER.search(c.get("body") or "")]
    if not markers:
        return ago(5)
    return (sp.when(max(markers)) + timedelta(seconds=1)).strftime("%Y-%m-%dT%H:%M:%SZ")


def decide(p):
    return sp.decide(p, NOW, DAYS)


class TriggerTests(unittest.TestCase):
    def test_conflicting_four_idle_days_warns(self):
        self.assertEqual(decide(pr(idle=4.1)), "warn")
        self.assertEqual(decide(pr(idle=3.9)), "skip")

    def test_a_persons_comment_counts_as_activity_and_a_bots_doesnt(self):
        self.assertEqual(decide(pr(idle=10, comments=[comment(ago(1))])), "skip")
        self.assertEqual(decide(pr(idle=10, comments=[comment(ago(1), bot=True)])), "warn")

    def test_a_reopen_counts_as_activity(self):
        p = pr(idle=10)
        p["timelineItems"]["nodes"].append({"__typename": "ReopenedEvent", "createdAt": ago(1)})
        self.assertEqual(decide(p), "skip")

    def test_a_push_counts_from_when_it_arrived_not_when_the_commit_was_made(self):
        self.assertEqual(decide(pr(idle=10, pushed=1)), "skip")
        # Fast-forward to an older SHA whose check suite was created days ago still updates suite updatedAt.
        self.assertEqual(decide(pr(idle=10, pushed=10, suite_updated=1)), "skip")

    def test_a_reply_in_a_review_thread_counts_as_activity(self):
        self.assertEqual(decide(pr(idle=10, threads=[{"publishedAt": ago(1), "author": {"__typename": "User"}}])),
                         "skip")
        self.assertEqual(decide(pr(idle=10, threads=[{"publishedAt": ago(1), "author": {"__typename": "Bot"}}])),
                         "warn")

    def test_asked_for_another_review_waits_for_the_reviewer(self):
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=20, review=(10, HEAD), requested=["Reviewer"])),
                         "skip")
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=20, review=(10, HEAD), requested=["someone"])),
                         "warn")

    def test_changes_requested_seven_days_on_the_same_head_warns(self):
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=20, review=(7.1, HEAD))), "warn")
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=20, review=(6.9, HEAD))), "skip")

    def test_a_push_after_the_review_waits_for_the_reviewer(self):
        # reviewDecision stays CHANGES_REQUESTED until the reviewer looks again: that's not on the author.
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=20, review=(10, OTHER))), "skip")

    def test_unknown_mergeable_is_never_acted_on(self):
        self.assertEqual(decide(pr(mergeable="UNKNOWN", idle=30)), "skip")
        self.assertEqual(decide(pr(mergeable="UNKNOWN", idle=30, review=(10, HEAD))), "skip")
        labelled = pr(mergeable="UNKNOWN", labels=["stale"], comments=[notice(ago(5))])
        self.assertEqual(decide(labelled), "wait")

    def test_clean_and_approved_is_left_alone(self):
        self.assertEqual(decide(pr(mergeable="MERGEABLE", idle=30)), "skip")

    def test_exclusions(self):
        for p in (pr(draft=True), pr(base="feature"), pr(queue=True), pr(labels=["Keep-Open"]),
                  pr(labels=["proposal"]), pr(labels=["tracker"]), pr(labels=["release-now"]), pr(labels=["pinned"])):
            self.assertEqual(decide(p), "skip")


class NoticeTests(unittest.TestCase):
    def test_conflict_cleared_by_a_push_clears(self):
        p = pr(labels=["stale"], comments=[notice(ago(2))], head=OTHER)
        self.assertEqual(decide(p), "clear")

    def test_conflict_resolved_by_main_clears(self):
        self.assertEqual(decide(pr(mergeable="MERGEABLE", labels=["stale"], comments=[notice(ago(2))])), "clear")

    def test_a_persons_comment_after_the_notice_clears(self):
        p = pr(labels=["stale"], comments=[notice(ago(3.5)), comment(ago(1))])
        self.assertEqual(decide(p), "clear")

    def test_a_persons_review_after_the_notice_clears_and_a_bots_doesnt(self):
        for bot, outcome in ((False, "clear"), (True, "close")):
            p = pr(labels=["stale"], comments=[notice(ago(4))])
            p["timelineItems"]["nodes"].append({"__typename": "PullRequestReview", "submittedAt": ago(1),
                                                "author": {"__typename": "Bot" if bot else "User"}})
            self.assertEqual(decide(p), outcome)

    def test_an_earlier_cycles_notice_doesnt_count(self):
        # Warned, cleared, labelled again without a fresh marker: left alone as unclear.
        self.assertEqual(decide(pr(labels=["stale"], comments=[notice(ago(9))], label_at=ago(4))), "unclear")
        self.assertEqual(decide(pr(labels=["stale"], comments=[notice(ago(4))], label_by="someone")), "unclear")

    def test_triage_bot_label_without_marker_is_left_alone(self):
        # `/label stale` via triage-command.yml uses github-actions without posting our marker: never auto-closes.
        self.assertEqual(decide(pr(labels=["stale"])), "unclear")
        self.assertEqual(decide(pr(mergeable="MERGEABLE", labels=["stale"])), "unclear")
        self.assertEqual(decide(pr(mergeable="UNKNOWN", labels=["stale"])), "unclear")

    def test_an_exclusion_label_or_draft_clears(self):
        self.assertEqual(decide(pr(labels=["stale", "keep-open"], comments=[notice(ago(5))])), "clear")
        self.assertEqual(decide(pr(draft=True, labels=["stale"], comments=[notice(ago(5))])), "clear")

    def test_closes_three_days_after_the_notice(self):
        self.assertEqual(decide(pr(labels=["stale"], comments=[notice(ago(3.1))])), "close")
        self.assertEqual(decide(pr(labels=["stale"], comments=[notice(ago(2.9))])), "wait")

    def test_bot_comments_after_the_notice_dont_stop_the_close(self):
        p = pr(labels=["stale"], comments=[notice(ago(4)), comment(ago(1), login="coderabbitai", bot=True)])
        self.assertEqual(decide(p), "close")

    def test_only_the_reason_named_closes(self):
        # Warned for the conflict, which went away; the review clock never had its own notice.
        p = pr(mergeable="MERGEABLE", review=(20, HEAD), labels=["stale"], comments=[notice(ago(4))])
        self.assertEqual(decide(p), "clear")
        p = pr(mergeable="MERGEABLE", review=(20, HEAD), labels=["stale"],
               comments=[notice(ago(4), reasons=(sp.CONFLICT, sp.CHANGES))])
        self.assertEqual(decide(p), "close")

    def test_a_label_a_person_added_is_left_alone(self):
        self.assertEqual(decide(pr(labels=["stale"], label_by="someone")), "unclear")
        self.assertEqual(decide(pr(labels=["stale"], label_by="someone", comments=[notice(ago(5))])), "unclear")

    def test_only_the_tokens_marker_counts(self):
        # Someone else's marker is ignored: a stale label without our own marker stays unclear, never closes.
        forged = notice(ago(5), login="mallory")
        forged["author"]["__typename"] = "User"
        self.assertEqual(decide(pr(labels=["stale"], comments=[forged], label_at=ago(5))), "unclear")
        other = notice(ago(5), login="other-bot")
        self.assertEqual(decide(pr(labels=["stale"], comments=[other], label_at=ago(5))), "unclear")

    def test_the_notice_names_the_reason_and_the_head(self):
        text = sp.notice_text("o/r", {sp.CHANGES}, HEAD, DAYS)
        self.assertIn("Changes were requested", text)
        self.assertNotIn("conflicts", text)
        self.assertIn("closed in 3 days", text)
        self.assertEqual(sp.MARKER.search(text).groups(), ("changes", HEAD))


class FakeApi:
    def __init__(self, prs, apply=True, fresh=None, fresh_after=None, labels=("stale",)):
        self.repo = "o/r"
        self.apply = apply
        self.prs = prs
        self.fresh = fresh or {}  # n -> the pull request as it reads once `fresh_after[n]` reads (default 1) are done
        self.fresh_after = fresh_after or {}
        self.reads = {}
        self._labels = set(labels)
        self.calls = []

    def labels(self):
        return self._labels

    def open_pulls(self):
        return self.prs

    def pull(self, n):
        self.reads[n] = self.reads.get(n, 0) + 1
        if self.reads[n] > self.fresh_after.get(n, 1) and n in self.fresh:
            return self.fresh[n]
        return next(p for p in self.prs if p["number"] == n)

    def label(self, n):
        self.calls.append(("label", n))

    def unlabel(self, n):
        self.calls.append(("unlabel", n))

    def comment(self, n, text):
        self.calls.append(("comment", n))

    def close(self, n):
        self.calls.append(("close", n))


def sweep(api, cap=30, waits=None, closed=None):
    return sp.sweep(api, DAYS, cap, now=NOW, wait=(waits.append if waits is not None else lambda s: None),
                    log=lambda line: None, closed=closed)


class SweepTests(unittest.TestCase):
    def mixed(self):
        return [pr(1, idle=10), pr(2, labels=["stale"], comments=[notice(ago(4))]),
                pr(3, mergeable="MERGEABLE", labels=["stale"], comments=[notice(ago(1))]), pr(4, idle=1)]

    def test_apply_acts_in_order(self):
        api = FakeApi(self.mixed())
        done = sweep(api)
        self.assertEqual({k: v for k, v in done.items() if k != "skip"}, {"clear": [3], "close": [2], "warn": [1]})
        self.assertEqual(api.calls, [("unlabel", 3), ("close", 2), ("comment", 2), ("comment", 1), ("label", 1)])

    def test_dry_run_writes_nothing(self):
        api = FakeApi(self.mixed(), apply=False)
        done = sweep(api)
        self.assertEqual(done["warn"], [1])
        self.assertEqual(api.calls, [])

    def test_the_real_api_refuses_writes_in_a_dry_run(self):
        with self.assertRaises(AssertionError):
            sp.Api("o/r", apply=False).comment(1, "x")

    def test_the_cap_holds_the_rest_for_the_next_run(self):
        api = FakeApi([pr(n, idle=10) for n in range(1, 6)])
        done = sweep(api, cap=2)
        self.assertEqual(done["warn"], [1, 2])
        self.assertEqual(done["warn next run"], [3, 4, 5])
        self.assertEqual(len(api.calls), 4)

    def test_a_close_reads_the_pull_request_again(self):
        listed = pr(2, labels=["stale"], comments=[notice(ago(4))])
        pushed = pr(2, labels=["stale"], comments=[notice(ago(4))], head=OTHER)
        api = FakeApi([listed], fresh={2: pushed})
        done = sweep(api)
        self.assertEqual(done["recheck"], [2])
        self.assertEqual(api.calls, [])

    def test_a_warning_reads_the_pull_request_again(self):
        for fresh in (pr(1, idle=10, draft=True), pr(1, idle=10, labels=["keep-open"]),
                      pr(1, idle=10, comments=[comment(ago(0.1))])):
            api = FakeApi([pr(1, idle=10)], fresh={1: fresh}, fresh_after={1: 0})
            self.assertEqual(sweep(api)["recheck"], [1])
            self.assertEqual(api.calls, [])

    def test_a_missing_label_after_notice_adds_only_the_label(self):
        # If comment succeeded and labelling failed on a prior run, next run adds only the label (no duplicate).
        api = FakeApi([pr(1, idle=10, comments=[notice(ago(1))])])
        self.assertEqual(sweep(api)["warn"], [1])
        self.assertEqual(api.calls, [("label", 1)])

    def test_truncated_comment_window_skips_warning_to_avoid_duplicates(self):
        api = FakeApi([pr(1, idle=10, has_prev_comments=True)])
        self.assertEqual(sweep(api)["recheck"], [1])
        self.assertEqual(api.calls, [])

    def test_closed_heads_are_handed_on_for_cancelling(self):
        closed = []
        sweep(FakeApi([pr(2, labels=["stale"], comments=[notice(ago(4))])]), closed=closed)
        self.assertEqual(closed, [{
            "number": 2,
            "branch": "topic",
            "repo": "fork/r",
            "head": HEAD,
            "closed_at": NOW.strftime("%Y-%m-%dT%H:%M:%SZ"),
        }])
        closed = []
        sweep(FakeApi([pr(2, labels=["stale"], comments=[notice(ago(4))])], apply=False), closed=closed)
        self.assertEqual(closed, [])

    def test_the_closed_heads_reach_the_workflow_output(self):
        def fake(api, days, cap, closed):
            closed.append({"branch": "b", "repo": "o/r"})
            raise RuntimeError("a later write failed")
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "out")
            with mock.patch.dict(os.environ, {"GITHUB_OUTPUT": out}), mock.patch.object(sp, "sweep", fake):
                with self.assertRaises(RuntimeError):
                    sp.main(["--apply"])
            with open(out, encoding="utf-8") as f:
                self.assertEqual(f.read(), 'closed=[{"branch": "b", "repo": "o/r"}]\n')

    def test_unknown_waits_once_and_reads_again(self):
        waits = []
        api = FakeApi([pr(1, mergeable="UNKNOWN")])
        sweep(api, waits=waits)
        self.assertEqual(waits, [sp.MERGEABLE_WAIT_S])
        waits.clear()
        sweep(FakeApi([pr(1, mergeable="UNKNOWN", draft=True)]), waits=waits)
        self.assertEqual(waits, [])

    def test_the_command_line(self):
        seen = []
        with mock.patch.object(sp, "sweep", side_effect=lambda api, days, cap, closed: seen.append((api.apply, days, cap))):
            sp.main([])
            sp.main(["--dry-run", "--conflict-days", "5"])
            sp.main(["--apply", "--max-actions", "50"])
        self.assertEqual(seen, [(False, DAYS, 30), (False, {**DAYS, sp.CONFLICT: 5}, 30), (True, DAYS, 50)])
        with self.assertRaises(SystemExit), mock.patch("sys.stderr"):
            sp.main(["--apply", "--dry-run"])

    def test_a_missing_label_stops_an_applied_run(self):
        with self.assertRaises(SystemExit):
            sweep(FakeApi([pr(1)], labels=()))
        sweep(FakeApi([pr(1)], apply=False, labels=()))  # a dry run doesn't need it


if __name__ == "__main__":
    unittest.main()
