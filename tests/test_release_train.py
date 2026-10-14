import datetime as dt
from pathlib import Path
import tempfile
import unittest

from tools import release_train as train

NOW = dt.datetime(2026, 10, 15, 12, 0, tzinfo=dt.timezone.utc)


def pr(number, title="Fix a thing", labels=(), paths=("crates/horizon-client/src/game.rs",), issues=(), author="ludovic111"):
    return {"number": number, "title": title, "labels": list(labels), "paths": list(paths), "issues": list(issues),
            "author": author, "url": f"https://github.com/ludovic111/horizon/pull/{number}"}


class ReleaseTrainTests(unittest.TestCase):
    def test_player_facing_paths(self):
        exporters = {"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobDataExporter.java"}
        self.assertTrue(train.player_facing("crates/horizon-client/src/game.rs", "game"))
        self.assertTrue(train.player_facing("Cargo.lock", "game"))
        self.assertTrue(train.player_facing("xtask/src/main.rs", "game"))
        self.assertTrue(train.player_facing(next(iter(exporters)), "game", exporters))
        self.assertFalse(train.player_facing("mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodOracle.java", "game", exporters))
        self.assertFalse(train.player_facing("crates/horizon-world/examples/probe.rs", "game"))
        self.assertFalse(train.player_facing("crates/horizon-client/README.md", "game"))
        self.assertFalse(train.player_facing("tools/test_food_parity.py", "game"))
        self.assertFalse(train.player_facing("launcher/src/main.rs", "game"))
        self.assertTrue(train.player_facing("launcher/src/main.rs", "launcher"))
        self.assertTrue(train.player_facing("crates/horizon-prepare/src/lib.rs", "launcher"))
        self.assertFalse(train.player_facing("launcher/links.json", "launcher"))
        self.assertFalse(train.player_facing("crates/horizon-client/src/game.rs", "launcher"))

    def test_the_exporters_are_those_horizon_prepare_names(self):
        found = train.exporters()
        self.assertIn(train.EXPORTERS + "VanillaMobDataExporter.java", found)
        self.assertTrue(all(path.endswith(".java") for path in found))

    def test_game_tags_count_up_within_a_day(self):
        today = dt.date(2026, 10, 15)
        self.assertEqual(train.next_game_tag(["v2026.10.14", "launcher-v0.1.4"], today), "v2026.10.15")
        self.assertEqual(train.next_game_tag(["v2026.10.15"], today), "v2026.10.15.1")
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.1", "v2026.10.15.2"], today), "v2026.10.15.3")

    def test_launcher_tags_bump_the_patch_or_the_minor(self):
        tags = ["launcher-v0.1.3", "launcher-v0.1.10", "v2026.10.15", "launcher-v0.1.4"]
        self.assertEqual(train.next_launcher_tag(tags, minor=False), "launcher-v0.1.11")
        self.assertEqual(train.next_launcher_tag(tags, minor=True), "launcher-v0.2.0")

    def test_decision(self):
        hour = dt.timedelta(hours=1)
        decide = lambda prs, age, **kw: train.decision("game", prs, NOW - age, NOW, **kw)[0]
        self.assertTrue(decide([pr(1)], 4 * hour))
        self.assertFalse(decide([pr(1)], 2 * hour), "within the cooldown")
        self.assertTrue(decide([pr(1, labels=["release-now"])], 0.1 * hour))
        self.assertFalse(decide([pr(1, paths=[])], 9 * hour), "nothing players run")
        self.assertFalse(decide([pr(1, labels=["skip-release"])], 9 * hour))
        self.assertFalse(decide([pr(1, labels=["release-now", "skip-release"])], 9 * hour))
        self.assertFalse(decide([pr(1)], 9 * hour, paused=True))
        self.assertTrue(decide([], 0.1 * hour, paused=True, force=True))
        self.assertTrue(train.decision("game", [pr(1)], None, NOW)[0], "the first release")

    def test_notes(self):
        prs = [pr(5, "Nether portals: vanilla timing and linking (fixes #94)", issues=[94]),
               pr(6, "Controllers: gilrs input layer and gameplay", labels=["feature"], author="cosmic"),
               pr(7, "Release folder in Rust", labels=["skip-release"]),
               pr(8, "Website footer", paths=[])]
        title, body = train.notes("game", "v2026.10.15.1", prs, ["- Sound cuts out after an hour."])
        self.assertEqual(title, "Horizon 2026.10.15.1: Controllers, nether portals")
        self.assertIn("### New\n- Controllers: gilrs input layer and gameplay (#6)", body)
        self.assertIn("### Fixed\n- Nether portals: vanilla timing and linking (#5, fixes #94)", body)
        self.assertIn("### Known issues\n- Sound cuts out after an hour.", body)
        self.assertNotIn("#7", body)
        self.assertNotIn("#8", body)
        self.assertIn("Thanks to @cosmic.", body)
        self.assertLess(body.index("### New"), body.index("### Fixed"))

    def test_a_long_release_is_named_after_its_first_titles(self):
        prs = [pr(n, f"Change number {n} with a fairly long title") for n in range(1, 10)]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertTrue(title.startswith("Horizon 2026.10.15: Change number 9"), title)
        self.assertTrue(title.endswith("more fixes"), title)
        self.assertLess(len(title), 100)

    def test_a_feature_split_over_several_pull_requests_is_named_once(self):
        prs = [pr(1, "Controllers: rumble", labels=["feature"]), pr(2, "Controllers: menus", labels=["feature"]),
               pr(3, "Maces: smash attacks"), pr(4, "Fix stuck keys when opening Options")]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertEqual(title, "Horizon 2026.10.15: Controllers, stuck keys when opening Options, maces")
        self.assertEqual(train.summary([], [pr(n, f"Change number {n} with a fairly long title") for n in range(4)]),
                         "Change number 3 with a fairly long title and 3 more fixes")

    def test_launcher_notes(self):
        title, body = train.notes("launcher", "launcher-v0.2.0", [pr(3, "Launcher: say the self-update succeeded")], [])
        self.assertEqual(title, "Horizon launcher 0.2.0")
        self.assertIn("### Fixed\n- Launcher: say the self-update succeeded (#3)", body)
        self.assertNotIn("Known issues", body)

    def test_known_issues_are_the_bullets_of_the_file(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            path.write_text("# Known issues\n\nSome text.\n\n- One.\n- Two, with **bold**.\n  continued\n")
            self.assertEqual(train.known_issues(path), ["- One.", "- Two, with **bold**."])
            self.assertEqual(train.known_issues(Path(temp, "missing.md")), [])


if __name__ == "__main__":
    unittest.main()
