import datetime as dt
import unittest

from tools import release_channels as channels
from tools import release_train as train

# Just after the daily promotion (PROMOTE_AT, 20:00 UTC): a beta is due when it had been out 6 hours by 20:00.
NOW = dt.datetime(2026, 10, 19, 20, 30, tzinfo=dt.timezone.utc)


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
        chosen, why = channels.choose([STABLE, release("v2026.10.19", 6)], [], NOW)
        self.assertIsNone(chosen, "out 5 h 30 min at 20:00")
        self.assertIn("the one at 2026-10-20 20:00 UTC takes it", why)
        late = channels.choose([STABLE, release("v2026.10.19", 0.5)], [], NOW)[1]
        self.assertIn("the one at 2026-10-20 20:00 UTC takes it", late, "out after 20:00: 6 h by the next one")

    def test_stable_moves_once_a_day_whenever_the_run_starts(self):
        hour = dt.timedelta(hours=1)
        releases = [STABLE, release("v2026.10.19", 9), release("v2026.10.19.01", 6)]
        # Before 20:00 nothing new is due, even what has been out 6 hours by now.
        before = channels.choose(releases, [], NOW - hour)
        self.assertIsNone(before[0], before[1])
        # The 20:00 run and a late one or a retry pick the same beta; once it is stable, the evening's later runs
        # promote nothing, though v2026.10.19.01 has been out 6 hours by then.
        for late in (0, 1, 3):
            self.assertEqual(channels.choose(releases, [], NOW + late * hour)[0]["tag_name"], "v2026.10.19")
        promoted = [release("v2026.10.19", 9, prerelease=False), releases[2]]
        self.assertIsNone(channels.choose(promoted, [], NOW + 3 * hour)[0])
        # A run GitHub skipped is caught up by the next one, at any hour before the next day's promotion.
        self.assertEqual(channels.choose(releases, [], NOW + 20 * hour)[0]["tag_name"], "v2026.10.19")
        # The next day's promotion takes the one that came out too late for today's.
        self.assertEqual(channels.choose(promoted, [], NOW + 24 * hour)[0]["tag_name"], "v2026.10.19.01")
        self.assertEqual(channels.promotion_time(NOW), dt.datetime(2026, 10, 19, 20, 0, tzinfo=dt.timezone.utc))
        self.assertEqual(channels.promotion_time(NOW - hour), dt.datetime(2026, 10, 18, 20, 0, tzinfo=dt.timezone.utc))

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

    def test_a_stall_is_reported_once_the_oldest_waiting_beta_missed_a_daily_promotion(self):
        # Out at 14:31 yesterday: just too late for yesterday's 20:00, so 29 h 59 min of waiting is a normal day.
        self.assertIsNone(channels.stalled([STABLE, release("v2026.10.19", 29.99), release("v2026.10.19.01", 2)], NOW))
        releases = [STABLE, release("v2026.10.19", 31), release("v2026.10.19.01", 2)]
        oldest, age = channels.stalled(releases, NOW)
        self.assertEqual(oldest["tag_name"], "v2026.10.19")
        title, body = channels.stall_issue(releases, (oldest, age), "no beta is due (release-blocker #4)")
        self.assertEqual(title, "Promotion to stable has stalled: v2026.10.19 has waited 31 h")
        self.assertIn("v2026.10.18.04", body)
        self.assertIn("release-blocker #4", body)
        # Betas older than the stable release are contained in it: they aren't waiting.
        self.assertIsNone(channels.stalled([STABLE, release("v2026.10.18.03", 40)], NOW))
        self.assertIsNone(channels.stalled([STABLE], NOW))

    def test_beta_notes_say_so_and_stable_notes_do_not(self):
        pr = {"number": 1, "title": "Fix a thing", "labels": [], "paths": ["crates/x.rs"], "issues": [], "author": "ludovic111"}
        _, beta = train.notes("game", "v2026.10.19", [pr], [], beta=True)
        _, stable = train.notes("game", "v2026.10.19", [pr], [], beta=False)
        self.assertIn("This is an alpha build: launchers on the Alpha channel", beta)
        self.assertNotIn("alpha", stable.lower())
        _, launcher = train.notes("launcher", "launcher-v0.9.0", [pr], [], beta=True)
        self.assertNotIn("alpha", launcher.lower())


if __name__ == "__main__":
    unittest.main()
