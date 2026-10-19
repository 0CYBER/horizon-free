import datetime as dt
import unittest

from tools import release_channels as channels
from tools import release_train as train

NOW = dt.datetime(2026, 10, 19, 18, 0, tzinfo=dt.timezone.utc)


def release(tag, hours_ago, prerelease=True, smoke=True, draft=False):
    published = (NOW - dt.timedelta(hours=hours_ago)).isoformat().replace("+00:00", "Z")
    assets = [{"name": "horizon-linux-x86_64.tar.gz"}] + ([{"name": channels.SMOKE_ASSET}] if smoke else [])
    return {"tag_name": tag, "published_at": published, "prerelease": prerelease, "draft": draft, "assets": assets}


def blocker(number, title, body="", state="open", reason=None, closed_hours_ago=None):
    closed = None if closed_hours_ago is None else (NOW - dt.timedelta(hours=closed_hours_ago)).isoformat()
    return {"number": number, "title": title, "body": body, "state": state, "state_reason": reason, "closed_at": closed}


STABLE = release("v2026.10.18.04", 30, prerelease=False)


class ReleaseChannelTests(unittest.TestCase):
    def test_the_newest_beta_out_six_hours_with_a_smoke_test_is_promoted(self):
        releases = [STABLE, release("v2026.10.19", 9), release("v2026.10.19.01", 7), release("v2026.10.19.02", 2)]
        chosen, why = channels.choose(releases, [], NOW)
        self.assertEqual(chosen["tag_name"], "v2026.10.19.01")
        self.assertIn("6 h", why)

    def test_nothing_is_due_before_six_hours(self):
        chosen, why = channels.choose([STABLE, release("v2026.10.19", 5.5)], [], NOW)
        self.assertIsNone(chosen)
        self.assertIn("0 h 30 min to go", why)

    def test_a_beta_without_its_smoke_test_waits_for_a_hand(self):
        releases = [STABLE, release("v2026.10.19", 8, smoke=False)]
        self.assertIsNone(channels.choose(releases, [], NOW)[0])
        self.assertIsNone(channels.choose(releases, [], NOW, tag="v2026.10.19")[0])
        self.assertEqual(channels.choose(releases, [], NOW, tag="v2026.10.19", force=True)[0]["tag_name"], "v2026.10.19")

    def test_by_hand_skips_the_wait_but_not_a_blocker(self):
        releases = [STABLE, release("v2026.10.19.02", 1)]
        self.assertEqual(channels.choose(releases, [], NOW, tag="v2026.10.19.02")[0]["tag_name"], "v2026.10.19.02")
        blocked = [blocker(5, "Crash on start in 2026.10.19.2")]
        chosen, why = channels.choose(releases, blocked, NOW, tag="v2026.10.19.02")
        self.assertIsNone(chosen)
        self.assertIn("#5", why)
        self.assertIsNotNone(channels.choose(releases, blocked, NOW, tag="v2026.10.19.02", force=True)[0])

    def test_stable_releases_and_drafts_are_never_promoted(self):
        self.assertIsNone(channels.choose([STABLE], [], NOW, tag=STABLE["tag_name"])[0])
        withdrawn = release("v2026.10.19", 9, draft=True)
        self.assertIsNone(channels.choose([STABLE, withdrawn], [], NOW)[0])
        older = release("v2026.10.18.03", 40)
        self.assertIsNone(channels.choose([STABLE, older], [], NOW)[0])

    def test_a_blocker_holds_the_version_it_names_and_every_later_beta(self):
        releases = [STABLE, release("v2026.10.19", 12), release("v2026.10.19.01", 10), release("v2026.10.19.02", 8)]
        chosen, _ = channels.choose(releases, [blocker(7, "Chests empty", "Since v2026.10.19.01 the chests…")], NOW)
        self.assertEqual(chosen["tag_name"], "v2026.10.19")

    def test_a_blocker_naming_no_version_holds_every_beta(self):
        releases = [STABLE, release("v2026.10.19", 12)]
        self.assertIsNone(channels.choose(releases, [blocker(8, "Something is badly wrong")], NOW)[0])

    def test_a_fixed_blocker_still_holds_the_betas_published_before_the_fix(self):
        fixed = blocker(9, "Crash in v2026.10.19", state="closed", reason="completed", closed_hours_ago=7.5)
        releases = [STABLE, release("v2026.10.19", 12), release("v2026.10.19.01", 7)]
        self.assertEqual(channels.choose(releases, [fixed], NOW)[0]["tag_name"], "v2026.10.19.01")
        self.assertIsNone(channels.choose(releases[:2], [fixed], NOW)[0])
        not_planned = blocker(10, "Crash in v2026.10.19", state="closed", reason="not_planned", closed_hours_ago=1)
        self.assertEqual(channels.choose(releases[:2], [not_planned], NOW)[0]["tag_name"], "v2026.10.19")

    def test_versions_people_write_in_issues(self):
        found = channels.mentioned_versions("v2026.10.19.03, 2026.10.18 and 2026.10.19.3; not Minecraft 1.21.10 or 26.3")
        self.assertEqual(found, {(2026, 10, 19, 3), (2026, 10, 18, 0)})

    def test_beta_notes_say_so_and_stable_notes_do_not(self):
        pr = {"number": 1, "title": "Fix a thing", "labels": [], "paths": ["crates/x.rs"], "issues": [], "author": "ludovic111"}
        _, beta = train.notes("game", "v2026.10.19", [pr], [], beta=True)
        _, stable = train.notes("game", "v2026.10.19", [pr], [], beta=False)
        self.assertIn("beta build", beta)
        self.assertNotIn("beta build", stable)
        _, launcher = train.notes("launcher", "launcher-v0.9.0", [pr], [], beta=True)
        self.assertNotIn("beta build", launcher)


if __name__ == "__main__":
    unittest.main()
