import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import pr_ready
from tools.ci import pr_labels as pl

ROOT = Path(__file__).resolve().parents[1]


def body(*ticked, other=""):
    lines = [f"- [{'x' if word in ticked else ' '}] {word}: something" for word, _, _ in pl.KINDS]
    return "## What and why\n\nStuff.\n\n## Kind\n\n" + "\n".join(lines) + f"\n\n## Self-check\n\n{other}"


def event(new, old=None, labels=(), action="edited"):
    ev = {"action": action, "pull_request": {"number": 9, "body": new, "labels": [{"name": n} for n in labels]}}
    if old is not None:
        ev["changes"] = {"body": {"from": old}}
    return ev


class KindTests(unittest.TestCase):
    def test_ticked(self):
        self.assertEqual(pl.ticked(body("parity")), ["parity"])
        self.assertEqual(pl.ticked(body("docs")), ["documentation"])
        self.assertEqual(pl.ticked(body()), [])
        self.assertEqual(pl.ticked(body("perf", "bug")), ["bug", "perf"])
        self.assertEqual(pl.ticked(None), [])

    def test_only_the_kind_section(self):
        self.assertEqual(pl.ticked(body(other="- [x] bug\n- [x] tooling")), [])
        self.assertEqual(pl.ticked("- [x] parity"), [])
        self.assertEqual(pl.ticked("## Kind\n* [X] **perf**: faster\n- [x] unknown\n"), ["perf"])

    def test_the_template_has_every_kind_unticked(self):
        text = (ROOT / ".github/pull_request_template.md").read_text()
        self.assertEqual(pl.ticked(text), [])
        for word, _, _ in pl.KINDS:
            self.assertIn(f"- [ ] {word}", text)

    def test_added_when_none_yet(self):
        self.assertEqual(pl.plan_event(event(body("parity"), action="opened"), []), (["parity"], None))

    def test_one_box_only(self):
        self.assertEqual(pl.plan_event(event(body("parity", "bug"), action="opened"), []), ([], None))

    def test_a_kind_set_by_hand_stays(self):
        self.assertEqual(pl.plan_event(event(body("parity"), body(), labels=("feature",)), None), ([], None))

    def test_changing_the_tick_swaps(self):
        self.assertEqual(pl.plan_event(event(body("bug"), body("parity"), labels=("parity",)), None),
                         (["bug"], "parity"))

    def test_changing_the_tick_never_drops_a_hand_label(self):
        self.assertEqual(pl.plan_event(event(body("bug"), body("parity"), labels=("parity", "feature")), None),
                         ([], None))
        self.assertEqual(pl.plan_event(event(body("bug"), body("parity"), labels=("feature",)), None), ([], None))

    def test_already_there(self):
        self.assertEqual(pl.plan_event(event(body("perf"), body(), labels=("perf",)), None), ([], None))


class AreaTests(unittest.TestCase):
    def test_paths(self):
        cases = {
            "launcher/src/main.rs": "launcher",
            "crates/horizon-world/src/worldgen/structure/mod.rs": "worldgen",
            "crates/horizon-client/src/mobs/ai.rs": "mobs",
            "crates/horizon-client/src/game/mobs/wither.rs": "mobs",
            "crates/horizon-client-render/src/gfx/cull.rs": "rendering",
            "crates/horizon-client-render/src/terrain.wgsl": "rendering",
            "crates/horizon-client/src/game/interface/dedicated/status.rs": "server",
            "crates/horizon-server-files/src/lib.rs": "server",
            "crates/horizon-client/src/lan/hub.rs": "multiplayer",
            "crates/horizon-client/src/game/network/remote_vehicles.rs": "multiplayer",
            "crates/horizon-protocol/src/packets/play.rs": "multiplayer",
            "crates/horizon-mod-package/src/lib.rs": "mods",
            "crates/horizon-client/src/game/native_hooks/mod.rs": "mods",
            "crates/horizon-world/src/datafix/v3.rs": "saves",
            "crates/horizon-client/src/game/administration/commands/admin.rs": "commands",
            "crates/horizon-command/src/args.rs": "commands",
            "crates/horizon-client-render/src/sound_engine.rs": "audio",
            "crates/horizon-client-render/src/streaming.rs": "chunks",
            "crates/horizon-world/src/level/redstone.rs": "blocks",
            "crates/horizon-client/src/title/worlds.rs": "ui",
            "crates/horizon-client-render/src/options_ui.rs": "ui",
            "docs/api/shaders.md": "website",
            "roadmap/roadmap.toml": "website",
        }
        for path, area in cases.items():
            self.assertEqual(pl.area_of(path), area, path)

    def test_no_area(self):
        for path in ("tools/parity.py", "crates/horizon-client/src/game.rs", "Cargo.lock", "docs/README.md",
                     "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobOracle.java",
                     "crates/horizon-client/examples/mob_ai_probe.rs"):
            self.assertIsNone(pl.area_of(path), path)

    def test_biggest_three_with_a_tenth_each(self):
        files = [("launcher/src/a.rs", 50), ("crates/horizon-client/src/mobs/a.rs", 30),
                 ("crates/horizon-command/src/a.rs", 10), ("crates/horizon-client-render/src/audio.rs", 9),
                 ("tools/x.py", 1)]
        self.assertEqual(pl.areas(files), ["launcher", "mobs", "commands"])
        self.assertEqual(pl.areas([("tools/x.py", 95), ("launcher/src/a.rs", 5)]), [])
        self.assertEqual(pl.areas([]), [])

    def test_files_count_on_push_not_on_edit(self):
        files = [("launcher/src/a.rs", 5)]
        self.assertEqual(pl.plan_event(event(body(), action="synchronize"), files), (["launcher"], None))
        self.assertEqual(pl.plan_event(event(body(), labels=("launcher",), action="synchronize"), files), ([], None))


class BackfillTests(unittest.TestCase):
    def test_guess_only_when_clear(self):
        self.assertEqual(pl.guess_kind("x", body("extra"), []), "extra")
        self.assertEqual(pl.guess_kind("x", None, [("docs/a.md", 3), ("README.md", 1)]), "documentation")
        self.assertEqual(pl.guess_kind("x", None, [("tools/a.py", 3), ("tests/test_a.py", 1)]), "tooling")
        self.assertEqual(pl.guess_kind("perf(client): less", None, [("launcher/a.rs", 1)]), "perf")
        self.assertEqual(pl.guess_kind("Fix a crash when", None, [("launcher/a.rs", 1)]), "bug")
        self.assertIsNone(pl.guess_kind("Mooshrooms keep their colour", None, [("crates/a.rs", 1)]))
        self.assertIsNone(pl.guess_kind("x", None, []))


class FakeApi:
    def __init__(self, labels=(), files=(), pulls=(), existing=()):
        self.now, self._files, self.pulls, self.existing = set(labels), list(files), list(pulls), set(existing)
        self.calls = []

    def files(self, n):
        return self._files

    def labels(self, n):
        return set(self.now)

    def add(self, n, names):
        self.calls.append(("add", n, names))

    def remove(self, n, name):
        self.calls.append(("remove", n, name))

    def open_pulls(self):
        return self.pulls

    def existing_labels(self):
        return self.existing

    def create_label(self, name, color, description):
        self.calls.append(("create", name))


class SizeTests(unittest.TestCase):
    def test_edges(self):
        for lines, size in ((0, "XS"), (50, "XS"), (51, "S"), (150, "S"), (151, "M"), (500, "M"), (501, "L"),
                            (3000, "L"), (3001, "XL")):
            self.assertEqual(pl.size_of([("crates/a/src/lib.rs", lines)]), f"size: {size}", lines)
        self.assertEqual(pl.size_of([]), "size: XS")
        self.assertEqual(pl.size_of([("crates/a/src/lib.rs", 2000), ("launcher/src/b.rs", 1001)]), "size: XL")

    def test_docs_and_tests_dont_count(self):
        files = [("crates/a/src/lib.rs", 40), ("docs/a.md", 5000), ("README.md", 900), ("tests/test_a.py", 900),
                 ("crates/a/tests/it.rs", 900), ("crates/a/src/test/x.rs", 900), ("tools/test_a_parity.py", 900)]
        self.assertEqual(pl.size_of(files), "size: XS")
        self.assertEqual(pl.size_of(files + [("Cargo.lock", 11)]), "size: S")  # pr_ready counts lock files too

    def test_counted_is_pr_readys_rule(self):
        for path in ("crates/a/src/lib.rs", "docs/a.md", "README.md", "tests/test_a.py", "crates/a/tests/it.rs",
                     "crates/a/src/test/x.rs", "tools/test_a_parity.py", "test_root.py", "tools/ci/pr_labels.py",
                     "Cargo.lock", "mods/horizon/src/test/java/A.java", "crates/a/src/tests.rs", "attest/a.rs"):
            self.assertEqual(pl.counted(path), pr_ready.reviewed(path), path)

    def test_one_size_replaces_the_old(self):
        files = [("crates/a/src/lib.rs", 200)]
        self.assertEqual(pl.size_change(set(), files), ("size: M", []))
        self.assertEqual(pl.size_change({"size: M", "bug"}, files), (None, []))
        self.assertEqual(pl.size_change({"size: S", "bug"}, files), ("size: M", ["size: S"]))
        self.assertEqual(pl.size_change({"size: XS", "size: M", "size: XL"}, files),
                         (None, ["size: XL", "size: XS"]))

    def run_event(self, api, action):
        ev = event(body("bug"), labels=("bug", "size: XS"), action=action)  # older than now
        with tempfile.TemporaryDirectory() as d:
            path = Path(d, "event.json")
            path.write_text(json.dumps(ev))
            with mock.patch.dict(os.environ, {"GITHUB_EVENT_PATH": str(path)}), mock.patch("builtins.print"):
                pl.event(api)

    def test_event_reads_the_labels_now(self):
        api = FakeApi(labels={"bug", "size: L"}, files=[("crates/a/src/lib.rs", 200)])
        self.run_event(api, "synchronize")
        self.assertEqual(api.calls, [("remove", 9, "size: L"), ("add", 9, ["size: M"])])

    def test_event_edit_leaves_size(self):
        api = FakeApi(labels={"bug", "size: L"}, files=[("crates/a/src/lib.rs", 200)])
        self.run_event(api, "edited")
        self.assertEqual(api.calls, [])

    def test_remove_encodes_the_name_and_ignores_a_missing_label(self):
        with mock.patch.object(pl, "gh") as gh:
            pl.Api("o/r").remove(7, "size: XS")
        gh.assert_called_once_with("api", "-X", "DELETE", "repos/o/r/issues/7/labels/size%3A%20XS")
        with mock.patch.object(pl, "gh", side_effect=RuntimeError("gh api : Label does not exist (HTTP 404)")):
            pl.Api("o/r").remove(7, "size: XS")
        with mock.patch.object(pl, "gh", side_effect=RuntimeError("gh api : (HTTP 403)")):
            self.assertRaises(RuntimeError, pl.Api("o/r").remove, 7, "size: XS")

    def test_create_only_what_is_missing(self):
        have = {label for _, label, _ in pl.KINDS} | pl.AREA_LABELS | {"Size: XS", "size: s"}
        api = FakeApi(existing=have)
        self.assertEqual(pl.create(api, apply=True), ["size: M", "size: L", "size: XL"])
        self.assertEqual(api.calls, [("create", "size: M"), ("create", "size: L"), ("create", "size: XL")])

    def test_backfill(self):
        pulls = [{"number": 1, "title": "x", "body": None, "labels": [{"name": "size: XL"}, {"name": "bug"}]},
                 {"number": 2, "title": "x", "body": None, "labels": [{"name": "size: XS"}, {"name": "bug"}]}]
        api = FakeApi(files=[("crates/a/src/lib.rs", 10)], pulls=pulls)
        self.assertEqual(pl.backfill(api, apply=False), [(1, ["size: XS"], ["size: XL"])])
        self.assertEqual(api.calls, [])
        pl.backfill(api, apply=True)
        self.assertEqual(api.calls, [("remove", 1, "size: XL"), ("add", 1, ["size: XS"])])


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/pr-labels.yml").read_text()
        run_lines = [l.strip() for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["run: python3 tools/ci/pr_labels.py event"])
        self.assertIn("pull_request_target", text)
        self.assertIn("ref: main", text)
        self.assertIn("persist-credentials: false", text)
        # Two pushes in a row run one after the other (one size label); edits run alone.
        self.assertIn("group: pr-labels-${{ github.event.pull_request.number }}-${{ github.event.action == 'edited' && "
                      "github.run_id || 'files' }}"
                      "\n      cancel-in-progress: false", text)
        for bad in ("contents: write", "issues: write", "head.sha", "head.ref", "github.event.pull_request.body"):
            self.assertNotIn(bad, text)
        # The checkout fetches the script alone.
        self.assertIn("sparse-checkout: |\n            tools/ci/pr_labels.py\n          sparse-checkout-cone-mode", text)


if __name__ == "__main__":
    unittest.main()
