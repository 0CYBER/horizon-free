import unittest

from tools.ci import parity_issues as issues


class ParityIssuesTests(unittest.TestCase):
    def test_results_merge_shards_and_skip_earlier_runs(self):
        state = {"harnesses": {
            "food": {"status": "pass", "finished": "2026-10-15T03:10:00"},
            "datafix#1": {"status": "pass", "finished": "2026-10-15T03:20:00"},
            "datafix#2": {"status": "fail", "finished": "2026-10-15T03:21:00"},
            "light": {"status": "fail", "finished": "2026-10-14T03:30:00"},
            "copied": {"status": "pass", "finished": "2026-10-15T03:30:00", "shared": True},
        }}
        self.assertEqual(issues.results(state, "2026-10-15T03:00:00"), {"food": "pass", "datafix": "fail"})

    def test_plan_opens_comments_and_closes(self):
        now = {"a": "fail", "b": "fail", "c": "pass", "d": "fail", "e": "pass"}
        previous = {"a": "pass", "b": "fail", "c": "fail", "e": "pass"}
        # A harness no audit ran before (d) gets an issue as one that passed does (a): #15158's nine merged failing
        # and no audit reported them. One that keeps passing (e) gets nothing.
        self.assertEqual(issues.plan(now, previous, {"b": 7, "c": 8}),
                         [("open", "a"), ("comment", "b"), ("close", "c"), ("open", "d")])
        # The first audit only records: it compares the results with themselves.
        self.assertEqual(issues.plan(now, now, {}), [])

    def test_many_new_failures_at_once_are_one_report(self):
        many = [("open", f"h{i}") for i in range(issues.MASS_FAILURE + 1)]
        self.assertEqual(issues.mass_failure(many + [("comment", "x"), ("close", "y")]),
                         [f"h{i}" for i in range(issues.MASS_FAILURE + 1)])
        # A few real regressions still get one issue each.
        self.assertEqual(issues.mass_failure([("open", "a"), ("open", "b"), ("comment", "c")]), [])

    def test_a_lasting_mass_failure_goes_on_the_open_report(self):
        listed = [{"number": 17794, "title": "Nightly audit: 57 harnesses newly fail"},
                  {"number": 16300, "title": "Parity regression: boat_void"},
                  {"number": 17343, "title": "Nightly audit: 58 harnesses newly fail"}]
        self.assertEqual(issues.mass_issue(listed), 17343)
        self.assertIsNone(issues.mass_issue(listed[1:2]))

    def test_no_mass_failure_closes_every_open_mass_issue(self):
        listed = [{"number": 5, "title": "Parity regression: x"},
                  {"number": 9, "title": "Nightly audit: 21 harnesses newly fail"},
                  {"number": 12, "title": "Nightly audit: 57 harnesses newly fail"}]
        # Two duplicates can be open at once (#17343/#17794): both close, not just the lowest-numbered one.
        self.assertEqual(issues.mass_issues(listed), [9, 12])
        calls = []

        def call(*arguments, dry_run=False):
            calls.append(arguments)

        self.assertEqual(issues.close_mass_issues(listed, "c" * 40, "run", call=call), [9, 12])
        self.assertEqual([c[0:2] for c in calls], [("issue", "close"), ("issue", "close")])
        self.assertEqual([c[2] for c in calls], ["9", "12"])
        self.assertIn("No mass failure", calls[0][4])
        # No open mass issue: nothing is closed.
        calls.clear()
        self.assertEqual(issues.close_mass_issues(listed[:1], "c" * 40, "run", call=call), [])
        self.assertEqual(calls, [])

    def test_mass_issues_close_only_after_a_finished_audit_with_history(self):
        self.assertFalse(issues.should_close_mass(None, True))
        self.assertFalse(issues.should_close_mass({"a": "fail"}, False))
        self.assertTrue(issues.should_close_mass({"a": "fail"}, True))


if __name__ == "__main__":
    unittest.main()
