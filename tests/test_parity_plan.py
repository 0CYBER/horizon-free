"""tools/parity_plan.py: shards grouped by the probe each harness builds, and the dependency lists a runner without
parity state of its own selects with."""
import argparse
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from tools import parity, parity_plan

WORLD = ["example", "world_probe", "release", ["-p", "horizon-world"], True]
CLIENT = ["example", "client_probe", "fast", ["-p", "horizon-client-lib"], True]
GAME = ["bins", "", "fast", ["-p", "horizon-game", "-p", "horizon-client-lib"], False]


class ProbeGroupTests(unittest.TestCase):
    def test_a_harness_joins_the_group_of_its_costliest_build(self):
        self.assertEqual(parity_plan.probe_group([WORLD, CLIENT, GAME]), "game")
        self.assertEqual(parity_plan.probe_group([["bin", "horizon-client", "fast", ["-p", "horizon-game"], False]]),
                         "game")
        self.assertEqual(parity_plan.probe_group([WORLD, CLIENT]), "client")
        self.assertEqual(parity_plan.probe_group([WORLD]), "world")
        self.assertEqual(parity_plan.probe_group([["example", "paletted_fixture", "fast", ["-p", "horizon-core"], True]]),
                         "other")
        self.assertEqual(parity_plan.probe_group([]), "other")

    def test_a_tools_binary_is_not_the_game(self):
        relay = ["bin", "horizon-relay", "dev", ["-p", "horizon-relay"], False]
        self.assertEqual(parity_plan.probe_group([relay]), "other")

    def test_the_shared_list_stands_in_for_a_harness_that_never_ran_here(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "SHARED", Path(temp)):
            (Path(temp) / "dependencies").mkdir()
            (Path(temp) / "dependencies/food.json").write_text(json.dumps({"dependencies": ["tools/lab.py"],
                                                                         "artifacts": [WORLD]}))
            records = {"light": {"status": "pass", "dependencies": ["tools/lab.py"], "artifacts": [CLIENT]}}
            self.assertEqual(parity_plan.probe_group(parity_plan.artifacts("food", records)), "world")
            self.assertEqual(parity_plan.probe_group(parity_plan.artifacts("light", records)), "client")
            self.assertEqual(parity_plan.probe_group(parity_plan.artifacts("new", records)), "other")


class PlanTests(unittest.TestCase):
    def setUp(self):
        self.names = [f"w{index:03}" for index in range(320)] + [f"c{index:03}" for index in range(120)] + \
            ["spawn#1", "spawn#2", "spawn#3", "spawn#4"] + ["g1", "g2"]
        self.groups = {name: "world" if name[0] in "ws" else "client" if name[0] == "c" else "game"
                       for name in self.names}

    def test_each_group_gets_the_shards_its_size_needs(self):
        count = parity_plan.shards_needed(self.names, self.groups)
        self.assertEqual(count, 3 + 2 + 1)  # 324 world harnesses at 150, 120 client at 100, 2 game at 50
        shards = parity_plan.plan(self.names, self.groups, count)
        self.assertEqual([group for group, _ in shards], ["game", "client", "client", "world", "world", "world"])
        for group, harnesses in shards:
            self.assertLessEqual(len(harnesses), parity_plan.SHARD_SIZES[group])
            self.assertEqual({self.groups[name] for name in harnesses}, {group})
        self.assertEqual(sorted(name for _, harnesses in shards for name in harnesses), sorted(self.names))

    def test_a_sharded_harnesss_entries_spread_over_its_groups_shards(self):
        shards = parity_plan.plan(self.names, self.groups, 6)
        holding = [index for index, (_, harnesses) in enumerate(shards) for name in harnesses if name.startswith("spawn")]
        self.assertEqual(len(set(holding)), 3)

    def test_fewer_shards_than_groups_put_whole_groups_together(self):
        shards = parity_plan.plan(self.names, self.groups, 2)
        self.assertEqual([group for group, _ in shards], ["world", "client+game"])
        self.assertEqual(shards[1][1], [name for name in self.names if self.groups[name] in ("client", "game")])

    def test_extra_shards_are_empty_and_the_same_split_comes_back(self):
        self.assertEqual(parity_plan.plan(["a", "b"], {"a": "world", "b": "world"}, 3),
                         [("world", ["a"]), ("world", ["b"]), ("world", [])])
        self.assertEqual(parity_plan.plan([], {}, 2), [("", []), ("", [])])
        self.assertEqual(parity_plan.plan(self.names, self.groups, 6), parity_plan.plan(self.names, self.groups, 6))

    def test_a_shard_spec_names_one_of_n(self):
        self.assertEqual(parity_plan.shard_spec("2/5"), (2, 5))
        for spec in ("0/5", "6/5", "1/0", "2", "a/b"):
            with self.subTest(spec=spec), self.assertRaises(argparse.ArgumentTypeError):
                parity_plan.shard_spec(spec)

    def test_the_selection_is_parity_pys_listed_names_and_what_it_left_out(self):
        output = "3 files changed: 2 harnesses depend on them, 2 without a pass for their current inputs.\n" \
                 "skipped on mini (darwin): save_roundtrip: needs vscreen\n" \
                 "not selected (runs when named, and nightly): lan: slow: 20 min\nfood\nlight\n2 of 9 harnesses would run\n"
        with mock.patch.object(parity_plan.subprocess, "run",
                               return_value=SimpleNamespace(returncode=0, stdout=output, stderr="")) as run, \
                mock.patch.object(parity, "harnesses", return_value={"food": 1, "light": 2, "lan": 3}):
            self.assertEqual(parity_plan.selection(["--changed", "origin/main"]),
                             (["food", "light"], {"save_roundtrip": "skipped on mini (darwin): needs vscreen",
                                                  "lan": "named only: slow: 20 min"}))
        self.assertEqual(run.call_args.args[0][-3:], ["--changed", "origin/main", "--list"])

    def test_the_left_out_lines_are_parity_pys_own(self):
        source = Path(parity.__file__).read_text(encoding="utf-8")
        self.assertIn('print(f"not selected (runs when named, and nightly): {name}: {manual[base_name(name)]}"', source)
        self.assertIn('print(f"skipped on {HOST} ({sys.platform}): {name}: {skips[base_name(name)]}"', source)


class MainTests(unittest.TestCase):
    def plan(self, names, left_out, needed=None):
        needs = {"return_value": needed} if needed else {"wraps": parity_plan.shards_needed}
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity_plan, "selection", return_value=(names, left_out)), \
                mock.patch.object(parity, "load_state", return_value={"harnesses": {}}), \
                mock.patch.object(parity_plan, "artifacts", return_value=[WORLD]), \
                mock.patch.object(parity_plan, "shards_needed", **needs), mock.patch("builtins.print"):
            path = Path(temp) / "plan.json"
            code = parity_plan.main(["--plan", str(path), "--", "--changed", "origin/main"])
            return code, json.loads(path.read_text(encoding="utf-8")) if path.exists() else None

    def test_the_plan_lists_what_was_left_out_and_why(self):
        code, written = self.plan(["food"], {"lan": "named only: slow"})
        self.assertEqual(code, 0)
        self.assertEqual(written, {"shards": [{"group": "world", "harnesses": ["food"]}],
                                   "left_out": {"lan": "named only: slow"}})

    def test_a_plan_beyond_githubs_matrix_limit_is_refused(self):
        self.assertEqual(self.plan(["food"], {}, needed=parity_plan.MATRIX_LIMIT + 1), (1, None))


class HostedTests(unittest.TestCase):
    def test_hosted_runners_leave_out_what_they_cannot_run_and_say_why(self):
        config = {"hosts": {"hosted": {"exclude": {"music": "no sounds"}}, "playhorizon": {"exclude": {"mob_ai": "heap"}}},
                  "known_failing": {"datafix": "#1: worlds"}, "named_only": {"lan": "slow"}}
        names = ["food", "music", "mob_ai", "datafix#1", "datafix#2", "lan"]
        self.assertEqual(parity_plan.hosted_left_out(names, config),
                         {"music": "no sounds", "datafix#1": "known failing: #1: worlds",
                          "datafix#2": "known failing: #1: worlds", "lan": "named only: slow"})

    def test_the_repositorys_hosted_excludes_name_real_harnesses(self):
        found = {parity.base_name(name) for name in parity.harnesses()}
        excluded = parity.host_config()["hosts"]["hosted"]["exclude"]
        self.assertEqual(set(excluded) - found, set())


class ExportTests(unittest.TestCase):
    def test_a_fresh_runs_passes_are_exported(self):
        records = {
            "food": {"status": "pass", "host": "runner", "seconds": 3, "hashes": {"tools/lab.py": "x"},
                     "dependencies": ["tools/lab.py", "crates/horizon-world/src/food.rs"], "artifacts": [WORLD],
                     "tests": [["-p", "horizon-world", "--lib"]]},
            "mace": {"status": "fail", "dependencies": ["tools/lab.py"], "artifacts": [GAME]},
            "gone": {"status": "pass", "dependencies": ["tools/lab.py"], "artifacts": []},
        }
        found = {name: None for name in ("food", "mace", "never_ran")}
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp) / "out"
            self.assertEqual(parity_plan.export_dependencies(records, found, directory), 1)
            self.assertEqual(sorted(path.name for path in directory.iterdir()), ["food.json"])
            self.assertEqual(json.loads((directory / "food.json").read_text(encoding="utf-8")),
                             {"dependencies": ["tools/lab.py", "crates/horizon-world/src/food.rs"],
                              "artifacts": [WORLD], "tests": [["-p", "horizon-world", "--lib"]]})

    def test_an_incomplete_export_writes_nothing(self):
        food = {"status": "pass", "dependencies": ["tools/lab.py"], "artifacts": [WORLD]}
        cases = {
            "shared": {"light": {"status": "pass", "shared": True, "dependencies": ["tools/lab.py"],
                                 "artifacts": [CLIENT]}},
            "no trace": {"hud": {"status": "pass", "dependencies": [], "artifacts": []}},
            "no probes": {"hud": {"status": "pass", "dependencies": ["tools/lab.py"]}},
            "not empty": {},
        }
        for case, records in cases.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temp:
                directory = Path(temp) / "out"
                if case == "not empty":
                    directory.mkdir()
                    (directory / "old.json").write_text("{}")
                records = {"food": food, **records}
                with self.assertRaises(parity_plan.IncompleteExport):
                    parity_plan.export_dependencies(records, dict.fromkeys(records), directory)
                self.assertFalse((directory / "food.json").exists())

    def test_a_failed_write_leaves_no_folder(self):
        records = {"food": {"status": "pass", "dependencies": ["tools/lab.py"], "artifacts": [WORLD]}}
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(Path, "write_text", side_effect=OSError("disk full")):
            directory = Path(temp) / "out"
            with self.assertRaises(parity_plan.IncompleteExport):
                parity_plan.export_dependencies(records, dict.fromkeys(records), directory)
            self.assertFalse(directory.exists())


if __name__ == "__main__":
    unittest.main()
