import datetime as dt
import unittest

from tools.ci import nightly

T = dt.datetime(2026, 10, 28, 12, 0, tzinfo=dt.timezone.utc)


def ref(name, sha, hours):
    return nightly.Ref(name, sha, T + dt.timedelta(hours=hours))


class NightlyTests(unittest.TestCase):
    def test_the_newest_union_newer_than_main_is_built(self):
        main = ref("main", "m", 0)
        unions = [ref("union/a", "a", 1), ref("union/b", "b", 3), ref("union/c", "c", -2)]
        self.assertEqual(nightly.choose(main, unions, contains=lambda a, b: False).name, "union/b")

    def test_main_when_no_union_is_newer(self):
        main = ref("main", "m", 5)
        unions = [ref("union/a", "a", 1), ref("union/b", "b", 3)]
        self.assertEqual(nightly.choose(main, unions, contains=lambda a, b: False).name, "main")
        self.assertEqual(nightly.choose(main, [], contains=lambda a, b: False).name, "main")

    def test_a_union_main_already_holds_is_skipped(self):
        main = ref("main", "m", 0)
        unions = [ref("union/landed", "l", 2), ref("union/open", "o", 1)]
        landed = lambda a, b: a == "l"  # noqa: E731
        self.assertEqual(nightly.choose(main, unions, contains=landed).name, "union/open")

    def test_only_old_dated_nightlies_are_pruned(self):
        tags = [{"tagName": "nightly", "isPrerelease": True},
                {"tagName": "nightly-2026-10-20", "isPrerelease": True},
                {"tagName": "nightly-2026-10-21", "isPrerelease": True},
                {"tagName": "nightly-2026-10-19", "isPrerelease": False},
                {"tagName": "v0.2.80", "isPrerelease": False},
                {"tagName": "nightly-2026-10-15-extra", "isPrerelease": True}]
        self.assertEqual(nightly.stale(tags, dt.date(2026, 10, 28), 7), ["nightly-2026-10-20"])

    def test_notes_list_what_is_not_on_main(self):
        text = nightly.notes("2026-10-28", ref("union/a", "0c0206714abc", 1),
                             [{"number": 2476, "title": "Fix pistons", "headRefOid": "x"}])
        self.assertIn("Nightly 2026-10-28 · 0c02067", text)
        self.assertIn("- Fix pistons (#2476)", text)
        self.assertIn("Everything in it is on main.", nightly.notes("2026-10-28", ref("main", "m" * 40, 0), []))


if __name__ == "__main__":
    unittest.main()
