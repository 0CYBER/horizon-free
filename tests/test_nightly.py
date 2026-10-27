import datetime as dt
import unittest
from pathlib import Path

from tools.ci import nightly
from tools.ci import nightly_set as ns

NOW = "2026-10-28T12:00:00+00:00"
PATTERNS = ns.guarded_patterns((Path(__file__).resolve().parents[1] / ".github/CODEOWNERS").read_text())


def pr(n, head="h", state="open", files=("crates/horizon-world/src/lib.rs",), base="main"):
    return ns.PullRequest(n, state, f"{head}{n}", f"Fix {n}", f"dev{n}", base, list(files))


def lookup(*prs):
    known = {p.number: p for p in prs}
    return lambda n: known.get(n)


def run(state, text, login="kacpherek", *prs):
    return ns.apply(state, ns.parse(text), login, NOW, lookup(*prs), PATTERNS)

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



class ScheduledTests(unittest.TestCase):
    def test_the_scheduled_nightly_takes_new_heads_and_drops_closed_ones(self):
        state = run({}, "/nightly 1 2 3 4", "kacpherek", pr(1), pr(2), pr(3), pr(4)).state
        get = lookup(pr(1, head="new"), pr(2, state="merged"), pr(3, state="closed"),
                     pr(4, files=[".github/workflows/x.yml"]))
        kept, left_out = nightly.refresh(state["prs"], get, PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "new1")])
        self.assertEqual(len(left_out), 3)
        self.assertIn("merged", left_out[0])

    def test_notes_list_each_pull_request_and_who_picked_it(self):
        prs = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2)).state["prs"]
        main = nightly.Ref("main", "m" * 40, None)
        text = nightly.set_notes("2026-10-28", nightly.Ref("nightly/build-2026-10-28-1", "b" * 40, None), main, prs,
                                 ["#3 Fix 3: closed, left out"])
        self.assertIn("can include pull requests not reviewed yet, picked by trusted reviewers", text)
        self.assertIn("- Fix 1 (#1) by dev1, at h1, picked by kacpherek", text)
        self.assertIn("### Left out (1)", text)

if __name__ == "__main__":
    unittest.main()
