import unittest
from unittest import mock
from datetime import datetime, timezone

from tools.ci import needs_info as ni

NOW = datetime(2026, 11, 20, 12, 0, tzinfo=timezone.utc)
LABELLED = "2026-11-13T11:00:00Z"  # 7 days and an hour before NOW


class FakeApi:
    def __init__(self, issues, unlabelled=()):
        self.issues = issues  # n -> (labelled_at, comments)
        self.unlabelled = set(unlabelled)  # listed, but the label was taken off before the issue's turn
        self.calls = []

    def has_label(self, n):
        return n not in self.unlabelled

    def labelled(self):
        return list(self.issues)

    def labelled_at(self, n):
        return self.issues[n][0]

    def comments_since(self, n, since):
        return [c for c in self.issues[n][1] if c["at"] >= since]

    def unlabel(self, n):
        self.calls.append(("unlabel", n))

    def close(self, n):
        self.calls.append(("close", n))


def comment(at, bot=False):
    return {"at": at, "bot": bot}


class DecideTests(unittest.TestCase):
    def test_no_answer_closes_after_seven_days(self):
        self.assertEqual(ni.decide(LABELLED, [], NOW), "close")
        self.assertIsNone(ni.decide("2026-11-14T12:00:00Z", [], NOW))

    def test_a_persons_comment_after_the_label_takes_it_off(self):
        self.assertEqual(ni.decide(LABELLED, [comment("2026-11-15T09:00:00Z")], NOW), "unlabel")

    def test_bot_comments_and_older_comments_change_nothing(self):
        # The triage command's confirmation, a seconds-later bot comment, and the comment that asked.
        self.assertEqual(ni.decide(LABELLED, [comment("2026-11-13T11:00:05Z", bot=True),
                                              comment("2026-11-13T10:59:00Z")], NOW), "close")

    def test_the_newest_labelling_counts(self):
        # Labelled again later: the week starts again.
        self.assertIsNone(ni.decide("2026-11-19T00:00:00Z", [comment("2026-11-15T09:00:00Z")], NOW))

    def test_no_label_event_waits(self):
        self.assertIsNone(ni.decide(None, [], NOW))


class RunTests(unittest.TestCase):
    def test_dry_run_changes_nothing_and_apply_acts(self):
        issues = {1: (LABELLED, []), 2: (LABELLED, [comment("2026-11-14T00:00:00Z")]),
                  3: ("2026-11-19T00:00:00Z", [])}
        api = FakeApi(issues)
        self.assertEqual(ni.run(api, False, NOW), ["#1 close", "#2 unlabel"])
        self.assertEqual(api.calls, [])
        ni.run(api, True, NOW)
        self.assertEqual(api.calls, [("close", 1), ("unlabel", 2)])

    def test_a_label_taken_off_after_the_listing_is_left_alone(self):
        api = FakeApi({1: (LABELLED, [])}, unlabelled={1})
        self.assertEqual(ni.run(api, True, NOW), [])
        self.assertEqual(api.calls, [])

    def test_close_closes_before_it_comments(self):
        calls = []
        with mock.patch.object(ni, "gh", side_effect=lambda *a: calls.append(a[:2]) or ""):
            ni.Api("o/r").close(5)
        self.assertEqual(calls, [("issue", "close"), ("issue", "comment")])


if __name__ == "__main__":
    unittest.main()
