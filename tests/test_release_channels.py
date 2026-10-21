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

    def test_an_open_blocker_holds_every_beta_whatever_its_text_says(self):
        releases = [STABLE, release("v2026.10.19", 12), release("v2026.10.19.01", 10), release("v2026.10.19.02", 8)]
        # The issue's author can edit its text after it is labelled: naming a later version must not free a beta.
        for text in ("Something is badly wrong", "Only in v2099.01.01", "Since v2026.10.19.01 the chests…"):
            chosen, why = channels.choose(releases, [blocker(7, text, body=text)], NOW)
            self.assertIsNone(chosen, text)
            self.assertIn("release-blocker #7", why)

    def test_a_fixed_blocker_still_holds_the_betas_published_before_the_fix(self):
        fixed = blocker(9, "Crash", state="closed", reason="completed", closed_hours_ago=7.5)
        releases = [STABLE, release("v2026.10.19", 12), release("v2026.10.19.01", 7)]
        self.assertEqual(channels.choose(releases, [fixed], NOW)[0]["tag_name"], "v2026.10.19.01")
        self.assertIsNone(channels.choose(releases[:2], [fixed], NOW)[0])
        not_planned = blocker(10, "Crash", state="closed", reason="not_planned", closed_hours_ago=1)
        self.assertEqual(channels.choose(releases[:2], [not_planned], NOW)[0]["tag_name"], "v2026.10.19")

    def test_force_never_moves_latest_backwards(self):
        # horizon-server update and install.sh download GitHub's latest: an older beta must never become it.
        older = release("v2026.10.18.03", 40, smoke=False)
        chosen, why = channels.choose([STABLE, older], [blocker(3, "x")], NOW, tag="v2026.10.18.03", force=True)
        self.assertIsNone(chosen)
        self.assertIn("not newer than the stable v2026.10.18.04", why)
        same = release("v2026.10.18.04", 30, prerelease=False)
        self.assertIsNone(channels.choose([same], [], NOW, tag="v2026.10.18.04", force=True)[0])

    def test_0_2_follows_the_dated_releases(self):
        bridge = release("v2026.10.21", 30, prerelease=False)
        first = release("v0.2.0", 7)
        chosen, why = channels.choose([bridge, first], [], NOW)
        self.assertEqual(chosen["tag_name"], "v0.2.0", why)
        stable = release("v0.2.0", 30, prerelease=False)
        older = release("v2026.10.22", 8)
        self.assertIsNone(channels.choose([stable, older], [], NOW)[0], "a dated beta never follows 0.2")
        chosen, why = channels.choose([stable, older], [], NOW, tag="v2026.10.22", force=True)
        self.assertIsNone(chosen)
        self.assertIn("not newer than the stable v0.2.0", why)
        patch = release("v0.2.1", 7)
        self.assertEqual(channels.choose([stable, older, patch], [], NOW)[0]["tag_name"], "v0.2.1")
        self.assertEqual(channels.stable([stable, bridge])["tag_name"], "v0.2.0")
        launcher = release("launcher@0.2.0", 7)
        self.assertIsNone(channels.choose([bridge, launcher], [], NOW)[0], "launcher releases are never game betas")

    def test_a_stall_is_reported_once_the_oldest_waiting_beta_is_a_day_old(self):
        self.assertIsNone(channels.stalled([STABLE, release("v2026.10.19", 23), release("v2026.10.19.01", 2)], NOW))
        releases = [STABLE, release("v2026.10.19", 25), release("v2026.10.19.01", 2)]
        oldest, age = channels.stalled(releases, NOW)
        self.assertEqual(oldest["tag_name"], "v2026.10.19")
        title, body = channels.stall_issue(releases, (oldest, age), "no beta is due (release-blocker #4)")
        self.assertEqual(title, "Promotion to stable has stalled: v2026.10.19 has waited 25 h")
        self.assertIn("v2026.10.18.04", body)
        self.assertIn("release-blocker #4", body)
        # Betas older than the stable release are contained in it: they aren't waiting.
        self.assertIsNone(channels.stalled([STABLE, release("v2026.10.18.03", 40)], NOW))
        self.assertIsNone(channels.stalled([STABLE], NOW))

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
