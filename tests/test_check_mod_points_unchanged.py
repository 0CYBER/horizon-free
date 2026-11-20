import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import check_mod_points_unchanged as gate


def point(name, target=None, hash_="aa", **extra):
    return {"name": name, "target": target or f"game::{name}", "signature_hash": hash_, **extra}


class Snapshots:
    """Two snapshot folders in a temporary folder, filled from points and file maps."""

    def __init__(self, test):
        self.tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self.tmp.cleanup)
        self.base, self.head = Path(self.tmp.name, "base"), Path(self.tmp.name, "head")

    def write(self, folder, points, files=None):
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "points.json").write_text(json.dumps(points))
        for name, text in (files or {}).items():
            (folder / name).parent.mkdir(parents=True, exist_ok=True)
            (folder / name).write_text(text)

    def run(self, base, head, base_files=None, head_files=None, **kwargs):
        self.write(self.base, base, base_files)
        self.write(self.head, head, head_files)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = gate.compare(self.base, self.head, **kwargs)
        return status, out.getvalue()


class DiffPointsTests(unittest.TestCase):
    def test_added_removed_and_changed(self):
        base = {"a": point("a"), "b": point("b"), "c": point("c", hash_="01")}
        head = {"a": point("a"), "c": point("c", hash_="02"), "d": point("d")}
        added, removed, changed = gate.diff_points(base, head)
        self.assertEqual(added, ["d"])
        self.assertEqual(removed, ["b"])
        self.assertEqual(changed, [("c", {"signature_hash": ("01", "02")})])

    def test_equal_catalogs(self):
        both = {"a": point("a", args=[["x", "i32"]])}
        self.assertEqual(gate.diff_points(both, dict(both)), ([], [], []))

    def test_a_moved_target_is_a_change(self):
        _, _, changed = gate.diff_points({"a": point("a", "game::old::a")}, {"a": point("a", "game::new::a")})
        self.assertEqual(changed, [("a", {"target": ("game::old::a", "game::new::a")})])

    def test_duplicate_names_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "points.json")
            path.write_text(json.dumps([point("a"), point("a")]))
            with self.assertRaises(ValueError):
                gate.load_points(path)


GAME = "horizon_client::game::Game<'_>::"
FREE = "horizon_client::game::helpers::"


class ClassifyTests(unittest.TestCase):
    def classify(self, removed, base, accepted=None, loops=(), files=None, changed=()):
        return gate.classify(removed, list(changed), base, accepted or {}, None if loops is None else set(loops), files or {})

    def test_a_listed_free_function_is_accepted(self):
        base = {"helper": point("helper", FREE + "helper")}
        ok, bad, _, _ = self.classify(["helper"], base, {"helper": ("#1", "moved to gameplay")})
        self.assertEqual((ok, bad), ({"helper": ("#1", "moved to gameplay")}, {}))

    def test_an_unlisted_removal_fails(self):
        base = {"helper": point("helper", FREE + "helper")}
        _, bad, _, _ = self.classify(["helper"], base)
        self.assertIn("not listed", bad["helper"])

    def test_a_game_method_fails_even_when_listed(self):
        for target in (GAME + "tick", "horizon_client::game::Game::tick"):
            base = {"tick": point("tick", target)}
            _, bad, _, _ = self.classify(["tick"], base, {"tick": ("#1", "x")})
            self.assertIn("`Game` method", bad["tick"])

    def test_a_type_that_only_starts_with_game_is_not_game(self):
        self.assertFalse(gate.is_game_method("horizon_client::game::GameRules::tick"))

    def test_a_loop_fails_even_when_listed(self):
        base = {"tick": point("tick", FREE + "tick_things")}
        _, bad, _, _ = self.classify(["tick"], base, {"tick": ("#1", "x")}, loops={gate.normalized(FREE + "tick_things")})
        self.assertIn("published loop", bad["tick"])

    def test_unknown_base_loops_fail_every_removal(self):
        base = {"helper": point("helper", FREE + "helper")}
        _, bad, _, _ = self.classify(["helper"], base, {"helper": ("#1", "x")}, loops=None)
        self.assertIn("no loops.json", bad["helper"])

    def test_a_longer_path_is_not_a_mention_of_the_target(self):
        base = {"helper": point("helper", FREE + "helper")}
        ok, bad, _, _ = self.classify(["helper"], base, {"helper": ("#1", "x")},
                                      files={"docs/x.md": f"see {FREE}helper_two"})
        self.assertEqual((list(ok), bad), (["helper"], {}))
        _, bad, _, _ = self.classify(["helper"], base, {"helper": ("#1", "x")}, files={"docs/x.md": f"see {FREE}helper."})
        self.assertIn("named in docs/x.md", bad["helper"])

    def test_a_fixture_doc_or_manifest_that_names_it_fails(self):
        base = {"helper_point": point("helper_point", FREE + "helper")}
        for text in ("listen at helper_point here", f"target = \"{FREE}helper\""):
            _, bad, _, _ = self.classify(["helper_point"], base, {"helper_point": ("#1", "x")},
                                         files={"docs/x.md": text})
            self.assertIn("named in docs/x.md", bad["helper_point"])

    def test_a_longer_name_is_not_a_mention(self):
        base = {"helper": point("helper", FREE + "helper")}
        ok, bad, _, _ = self.classify(["helper"], base, {"helper": ("#1", "x")},
                                      files={"docs/x.md": "the helper_two point and other::helper"})
        self.assertEqual((list(ok), bad), (["helper"], {}))

    def test_a_changed_point_passes_only_when_listed(self):
        fields = {"signature_hash": ("01", "02")}
        _, _, ok, bad = self.classify([], {}, {"a": ("#2", "arg renamed")}, changed=[("a", fields), ("b", fields)])
        self.assertEqual((list(ok), list(bad)), (["a"], ["b"]))


class AcceptedFileTests(unittest.TestCase):
    def read(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "list.txt")
            path.write_text(text)
            return gate.load_accepted(path)

    def test_lines_and_comments(self):
        got = self.read("# header\n\nhelper | #18198 | moved to gameplay\nother|#2|why | with a bar\n")
        self.assertEqual(got, {"helper": ("#18198", "moved to gameplay"), "other": ("#2", "why | with a bar")})

    def test_a_line_without_reason_is_refused(self):
        with self.assertRaises(ValueError):
            self.read("helper | #1\n")

    def test_the_checked_in_file_parses(self):
        self.assertIsInstance(gate.load_accepted(gate.ROOT / gate.LIST), dict)


class VerdictTests(unittest.TestCase):
    def verdict(self, added=(), removed_bad=None, changed_bad=None, files_added=(), files_removed=(), diffs=(),
                explained=False, allow_added=False):
        return gate.verdict(list(added), removed_bad or {}, changed_bad or {}, list(files_added), list(files_removed),
                            list(diffs), explained, allow_added)

    def test_no_difference_passes(self):
        self.assertEqual(self.verdict(), 0)

    def test_a_generated_diff_needs_an_accepted_point_change(self):
        self.assertEqual(self.verdict(diffs=[("lib.rs", ["-x"])]), 1)
        self.assertEqual(self.verdict(diffs=[("lib.rs", ["-x"])], explained=True), 0)

    def test_added_points_need_the_flag(self):
        self.assertEqual(self.verdict(added=["a"]), 1)
        self.assertEqual(self.verdict(added=["a"], diffs=[("lib.rs", ["+x"])], explained=True, allow_added=True), 0)

    def test_failures_always_fail(self):
        self.assertEqual(self.verdict(removed_bad={"a": "x"}, explained=True, allow_added=True), 1)
        self.assertEqual(self.verdict(changed_bad={"a": "x"}, explained=True, allow_added=True), 1)

    def test_a_strict_file_the_head_dropped_fails_even_when_explained(self):
        self.assertEqual(self.verdict(files_removed=["internals.json"], explained=True, allow_added=True), 1)

    def test_loops_are_strict(self):
        self.assertEqual(self.verdict(diffs=[("loops.json", ["-x"])], explained=True, allow_added=True), 1)

    def test_internals_pass_additions_and_changes_with_the_flag_only(self):
        added_changed = ([("x", "fn")], [], [("y", "struct")])
        self.assertEqual(gate.verdict([], {}, {}, [], [], [], False, True, added_changed), 0)
        self.assertEqual(gate.verdict([], {}, {}, [], [], [], True, False, added_changed), 1)
        self.assertEqual(gate.verdict([], {}, {}, [], [], [], True, True, ([], [("z", "fn")], [])), 1)


NO_LOOPS = json.dumps({"loops": []})


def internals(*entries):
    return json.dumps({"internals": [{"path": path, "kind": kind, "hash": hash_, "signature": path, **extra}
                                      for path, kind, hash_, extra in entries]})


A_FN = ("game::a", "fn", "01", {})
B_STRUCT = ("game::B", "struct", "02", {})


class CompareTests(unittest.TestCase):
    def run_compare(self, base, head, base_files=None, head_files=None, accepted=None, files=None, **kwargs):
        snaps = Snapshots(self)
        return snaps.run(base, head, base_files, head_files, accepted=accepted or {},
                         files=lambda: files or {}, **kwargs)

    def test_same_output_passes_and_prints_counts(self):
        status, text = self.run_compare([point("a")], [point("a")], {"api/lib.rs": "x"}, {"api/lib.rs": "x"})
        self.assertEqual(status, 0)
        self.assertIn("Mod points unchanged: 1 points", text)
        self.assertIn("0 added, 0 removed (0 accepted, 0 failing), 0 changed (0 accepted, 0 failing)", text)

    def test_unlisted_removal_fails_with_its_name_and_hash(self):
        status, text = self.run_compare([point("a"), point("b", FREE + "b", hash_="beef")], [point("a")])
        self.assertEqual(status, 1)
        self.assertIn("Points removed and failing (1):", text)
        self.assertIn("b  target " + FREE + "b  hash beef", text)
        self.assertIn("1 removed (0 accepted, 1 failing)", text)

    def test_listed_removal_passes_with_its_generated_diff(self):
        status, text = self.run_compare([point("a"), point("b", FREE + "b")], [point("a")],
                                        {"api.rs": "1", "loops.json": NO_LOOPS}, {"api.rs": "2", "loops.json": NO_LOOPS},
                                        accepted={"b": ("#9", "moved")})
        self.assertEqual(status, 0)
        self.assertIn("Points removed and listed (accepted) (1):", text)
        self.assertIn("1 removed (1 accepted, 0 failing)", text)

    def test_a_removal_fails_when_the_base_wrote_no_loops_json(self):
        status, text = self.run_compare([point("a"), point("b", FREE + "b")], [point("a")], {"api.rs": "1"},
                                        {"api.rs": "2", "loops.json": NO_LOOPS}, accepted={"b": ("#9", "moved")})
        self.assertEqual(status, 1)
        self.assertIn("no loops.json", text)

    def test_a_strict_file_the_head_dropped_fails_with_an_allowed_point(self):
        status, text = self.run_compare([point("a")], [point("a"), point("b")], {"internals.json": "{}"}, {"x": "1"},
                                        allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("the head has to write it and internals.json", text)

    def test_removed_game_method_fails_though_listed(self):
        status, text = self.run_compare([point("a", GAME + "a")], [], accepted={"a": ("#9", "moved")})
        self.assertEqual(status, 1)
        self.assertIn("a `Game` method", text)

    def test_removed_loop_entry_fails(self):
        loops = json.dumps({"loops": [{"name": "x.tick", "entry": FREE + "tick", "generic": "tick_with",
                                       "slot": "S", "side": "client"}]})
        status, _ = self.run_compare([point("t", FREE + "tick")], [], {"loops.json": loops}, {"loops.json": loops},
                                     accepted={"t": ("#9", "moved")})
        self.assertEqual(status, 1)

    def test_a_loops_json_difference_fails_even_with_an_accepted_removal(self):
        status, _ = self.run_compare([point("b", FREE + "b")], [], {"loops.json": "{}"}, {"loops.json": "{ }"},
                                     accepted={"b": ("#9", "moved")})
        self.assertEqual(status, 1)

    def test_internals_added_only_pass_with_the_flag(self):
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN)},
                                        {"internals.json": internals(A_FN, B_STRUCT)}, allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("Internals added (1):", text)
        self.assertIn("game::B (struct)", text)

    def test_internals_changed_pass_with_the_flag(self):
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN, B_STRUCT)},
                                        {"internals.json": internals(A_FN, ("game::B", "struct", "03", {}))},
                                        allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("Internals changed (1):", text)

    def test_an_internals_removal_fails_with_the_flag(self):
        status, text = self.run_compare([point("a")], [point("a"), point("b")],
                                        {"internals.json": internals(A_FN, B_STRUCT), "api.rs": "1"},
                                        {"internals.json": internals(A_FN), "api.rs": "2"}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed (1):", text)
        self.assertIn("never remove one", text)

    def test_one_of_two_kinds_on_a_path_removed_fails(self):
        field, method = ("game::S::x", "field", "01", {}), ("game::S::x", "method", "02", {})
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(field, method)},
                                        {"internals.json": internals(method)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("game::S::x (field)", text)

    def test_a_platform_swap_with_the_same_count_fails(self):
        unix, windows, mac = (("game::f", "fn", "01", {"cfg": cfg}) for cfg in ("unix", "windows", "macos"))
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(unix, windows)},
                                        {"internals.json": internals(unix, mac)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed (1):", text)
        self.assertIn("game::f (fn, cfg windows)", text)

    def test_becoming_shape_only_fails(self):
        exposed, shape = ("game::T", "struct", "01", {}), ("game::T", "struct", "01", {"shape_only": True})
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(exposed)},
                                        {"internals.json": internals(shape)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed (1):", text)
        status, _ = self.run_compare([point("a")], [point("a")], {"internals.json": internals(shape)},
                                     {"internals.json": internals(exposed)}, allow_added=True)
        self.assertEqual(status, 0)

    def test_an_unparsable_internals_difference_fails_with_status_1(self):
        for allow_added in (False, True):
            status, text = self.run_compare([point("a")], [point("a")], {"internals.json": "{}"},
                                            {"internals.json": "{ }"}, allow_added=allow_added)
            self.assertEqual(status, 1)
            self.assertIn("internals.json differs and does not parse as entries (ValueError: no `internals` array",
                          text)

    def test_an_unparsable_internals_keeps_the_other_diffs(self):
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": "{}", "loops.json": NO_LOOPS},
                                        {"internals.json": "{ }", "loops.json": json.dumps({"loops": [], "x": 1})},
                                        allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Generated files changed (2): internals.json, loops.json", text)
        self.assertIn("internals.json differs and does not parse as entries", text)
        self.assertIn('+{"loops": [], "x": 1}', text)

    def test_a_dropped_hash_fails(self):
        without_hash = json.loads(internals(A_FN))
        del without_hash["internals"][0]["hash"]
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN)},
                                        {"internals.json": json.dumps(without_hash)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("has no str `hash`", text)

    def test_wrongly_typed_optional_fields_fail(self):
        for extra in ({"cfg": 1}, {"shape_only": "yes"}):
            status, _ = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN)},
                                         {"internals.json": internals(A_FN[:3] + (extra,))}, allow_added=True)
            self.assertEqual(status, 1)

    def test_one_of_two_cfg_variants_removed_fails(self):
        on, off = ("game::f", "mod", "01", {"cfg": "feature = \"f\""}), ("game::f", "mod", "01", {"cfg": "not f"})
        status, _ = self.run_compare([point("a")], [point("a")], {"internals.json": internals(on, off)},
                                     {"internals.json": internals(on)}, allow_added=True)
        self.assertEqual(status, 1)

    def test_any_internals_difference_fails_without_the_flag(self):
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN)},
                                        {"internals.json": internals(A_FN, B_STRUCT)})
        self.assertEqual(status, 1)
        self.assertIn("Internals added (1):", text)

    def test_internals_changes_do_not_explain_other_generated_diffs(self):
        status, text = self.run_compare([point("a")], [point("a")], {"internals.json": internals(A_FN), "api.rs": "1"},
                                        {"internals.json": internals(A_FN, B_STRUCT), "api.rs": "2"},
                                        allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("no accepted point change", text)

    def test_a_loops_json_difference_fails_with_the_flag(self):
        status, _ = self.run_compare([point("a")], [point("a"), point("b")], {"loops.json": NO_LOOPS},
                                     {"loops.json": json.dumps({"loops": [], "x": 1})}, allow_added=True)
        self.assertEqual(status, 1)

    def test_a_strict_file_only_the_head_wrote_is_not_compared(self):
        status, text = self.run_compare([point("a")], [point("a")], None, {"loops.json": "{}"})
        self.assertEqual(status, 0)
        self.assertIn("loops.json exists on one side only", text)

    def test_changed_hash_is_shown_old_to_new_and_fails_unlisted(self):
        status, text = self.run_compare([point("a", hash_="01")], [point("a", hash_="02")])
        self.assertEqual(status, 1)
        self.assertIn("signature_hash: 01 -> 02", text)

    def test_listed_change_passes(self):
        status, text = self.run_compare([point("a", hash_="01")], [point("a", hash_="02")],
                                        accepted={"a": ("#3", "renamed argument")})
        self.assertEqual(status, 0)
        self.assertIn("1 changed (1 accepted, 0 failing)", text)

    def test_file_diff_is_printed_and_capped(self):
        old = "\n".join(f"line {i}" for i in range(500))
        new = "\n".join(f"other {i}" for i in range(500))
        status, text = self.run_compare([point("a")], [point("a")], {"catalog.json": old}, {"catalog.json": new})
        self.assertEqual(status, 1)
        self.assertIn("Generated files changed (1): catalog.json", text)
        self.assertIn("more lines of catalog.json left out", text)
        self.assertLess(len(text.splitlines()), gate.MAX_DIFF_LINES + 25)

    def test_allow_added_passes_with_a_new_point_and_its_generated_code(self):
        status, text = self.run_compare([point("a")], [point("a"), point("b")], {"api.rs": "1"}, {"api.rs": "2"},
                                        allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("1 added, 0 removed", text)

    def test_added_points_fail_without_the_flag(self):
        status, text = self.run_compare([point("a")], [point("a"), point("b")])
        self.assertEqual(status, 1)
        self.assertIn("without --allow-added", text)

    def test_generator_scratch_is_not_compared(self):
        base = {"interfaces-aaa/1.cache": "x", "points-timing.txt": "a 1.00s\n", "api.rs": "1"}
        head = {"interfaces-bbb/2.cache": "y", "interfaces-bbb/3.cache": "z", "incremental-ccc/4.entry": "w",
                "points-timing.txt": "a 2.00s\n", "api.rs": "1"}
        status, text = self.run_compare([point("a")], [point("a")], base, head)
        self.assertEqual(status, 0)
        self.assertIn("Mod points unchanged", text)

    def test_only_the_cache_folder_and_timing_file_are_scratch(self):
        status, _ = self.run_compare([point("a")], [point("a")], {"api/points-timing.txt": "1"},
                                     {"api/points-timing.txt": "2"})
        self.assertEqual(status, 0)
        status, _ = self.run_compare([point("a")], [point("a")], {"interfaces.rs": "1"}, {"interfaces.rs": "2"})
        self.assertEqual(status, 1)

    def test_source_folder_paths_do_not_count_as_a_difference(self):
        status, _ = self.run_compare([point("a")], [point("a")], {"hook-notes.md": "see /tmp/base/x"},
                                     {"hook-notes.md": "see /work/head/x"}, base_root="/tmp/base",
                                     head_root="/work/head")
        self.assertEqual(status, 0)

    def test_summary_lists_are_capped(self):
        head = {f"p{i:03}": point(f"p{i:03}") for i in range(100)}
        lines = gate.summary(sorted(head), {}, {}, [], {}, [], [], [], {}, head)
        self.assertIn("Points added (100):", lines[0])
        self.assertIn(f"... and {100 - gate.MAX_LISTED} more", lines[-1])


class RepoTextsTests(unittest.TestCase):
    def test_a_failed_git_listing_is_an_error_not_an_empty_map(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(RuntimeError):
                gate.repo_texts(tmp)


class TargetFolderTests(unittest.TestCase):
    def test_a_relative_folder_becomes_one_absolute_path(self):
        for option, env in ((Path("target/mod-points"), None), (None, "target/x")):
            self.assertTrue(gate.resolve_target(option, env).is_absolute())
        self.assertEqual(gate.resolve_target(Path("a"), "b"), Path("a").resolve())
        self.assertEqual(gate.resolve_target(None, None), (gate.ROOT / "target").resolve())

    def test_the_base_builds_in_its_own_folder_inside_the_working_trees(self):
        target = Path("/build/target")
        self.assertEqual(gate.base_target(target), target / "mod-points-base")
        self.assertNotEqual(gate.base_target(target), target)


class SnapshotStoreTests(unittest.TestCase):
    """A stored snapshot of the base stands in for its build only when complete and under the same key."""

    RUSTC = "rustc 1.99.0 (abc 2026-11-01)\nhost: x86_64-unknown-linux-gnu"

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.store = self.tmp / "store"
        self.key = gate.snapshot_key("c0ffee", self.RUSTC, env={})

    def made(self, name, root, points=None):
        """A snapshot folder as hook_snapshot writes it, naming `root` in a generated file."""
        out = self.tmp / name
        out.mkdir()
        (out / "points.json").write_text(json.dumps(points or [point("tick")]))
        (out / "api.rs").write_text(f"// generated from {root}/crates/horizon-client/src/game.rs\n")
        return out

    def stored(self, key=None):
        self.made("main-out", "/work/main")
        gate.store_snapshot(self.store, self.tmp / "main-out", key or self.key, "/work/main")
        return self.store / (key or self.key)["commit"]

    def test_a_stored_snapshot_compares_as_the_built_base_would(self):
        self.stored()
        folder, root, route = gate.find_snapshot(self.store, self.key)
        self.assertEqual(route, "hit")
        self.assertEqual(root, "/work/main")
        head = self.made("head-out", "/work/head")
        built = self.made("built-out", "/tmp/base")
        with contextlib.redirect_stdout(io.StringIO()) as hit:
            hit_status = gate.compare(folder, head, base_root=root, head_root="/work/head")
        with contextlib.redirect_stdout(io.StringIO()) as build:
            build_status = gate.compare(built, head, base_root="/tmp/base", head_root="/work/head")
        self.assertEqual((hit_status, hit.getvalue()), (build_status, build.getvalue()))
        self.assertEqual(hit_status, 0)

    def test_nothing_stored_is_a_miss(self):
        self.assertEqual(gate.find_snapshot(self.store, self.key), (None, None, "miss: none stored"))

    def test_another_toolchain_or_point_selection_is_a_miss(self):
        self.stored()
        for key, differs in ((gate.snapshot_key("c0ffee", "rustc 2.0.0", env={}), "rustc"),
                             (gate.snapshot_key("c0ffee", self.RUSTC, env={"HORIZON_POINT_NAMES": '["tick"]'}),
                              "point_names")):
            self.assertEqual(gate.find_snapshot(self.store, key),
                             (None, None, f"miss: stored under another key ({differs})"))

    def remarked(self, folder, digest=True, **fields):
        """Rewrite the marker with `fields` changed, its digest recomputed or (digest=False) left as it was."""
        marker = json.loads((folder / gate.MARKER).read_text())
        marker.update(fields)
        if digest:
            marker["digest"] = gate.marker_digest(marker["key"], marker["root"], marker["files"])
        (folder / gate.MARKER).write_text(json.dumps(marker))

    def test_an_incomplete_or_corrupt_snapshot_is_a_miss_not_an_error(self):
        damages = {
            "truncated file": lambda folder: (folder / "out/points.json").write_text("[{"),
            "added file": lambda folder: (folder / "out/extra.json").write_text("{}"),
            "removed file": lambda folder: (folder / "out/api.rs").unlink(),
            "no marker": lambda folder: (folder / gate.MARKER).unlink(),
            "garbled marker": lambda folder: (folder / gate.MARKER).write_text("{\"key\": "),
            "damaged root": lambda folder: self.remarked(folder, digest=False, root="/work/mail"),
            "damaged key": lambda folder: self.remarked(folder, digest=False, key=dict(self.key, rustc="x")),
            "marker of another commit": lambda folder: self.remarked(folder, key=dict(self.key, commit="beef")),
            "marker without digest": lambda folder: (folder / gate.MARKER).write_text(
                json.dumps({"key": self.key, "root": "/work/main", "files": gate.file_hashes(folder / "out")})),
        }
        for name, damage in damages.items():
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                damage(self.stored())
                folder, root, route = gate.find_snapshot(self.store, self.key)
                self.assertIsNone(folder)
                self.assertTrue(route.startswith("miss: "), route)

    def test_only_the_newest_snapshots_and_no_partial_ones_are_kept(self):
        for number in range(4):
            out = self.tmp / f"out{number}"
            out.mkdir()
            (out / "points.json").write_text("[]")
            gate.store_snapshot(self.store, out, dict(self.key, commit=f"c{number}"), "/r", keep=10)
            marker = self.store / f"c{number}" / gate.MARKER
            os.utime(marker, (1000 + number, 1000 + number))
        (self.store / f"{gate.TEMPORARY}c9-1").mkdir()
        gate.prune(self.store, keep=2)
        self.assertEqual(sorted(path.name for path in self.store.iterdir()), ["c2", "c3"])

    def test_a_pull_request_never_stores(self):
        self.assertFalse(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "pull_request"}))
        self.assertFalse(gate.may_store({"SAVE_BUILD_DISK": "false", "GITHUB_EVENT_NAME": "workflow_dispatch"}))
        self.assertFalse(gate.may_store({}))
        self.assertTrue(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "push"}))
        self.assertTrue(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "merge_group"}))
        built = mock.Mock(side_effect=AssertionError("built"))
        with mock.patch.dict(os.environ, {"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "pull_request"}), \
                mock.patch.object(gate, "cargo", built), contextlib.redirect_stdout(io.StringIO()):
            status = gate.main(["--store-snapshot", str(self.store)])
        self.assertEqual(status, 2)
        self.assertFalse(self.store.exists())

    def test_a_failed_store_leaves_no_partial_copy(self):
        out = self.made("main-out", "/work/main")
        with mock.patch.object(gate, "file_hashes", mock.Mock(side_effect=OSError("No space left on device"))):
            with self.assertRaises(OSError):
                gate.store_snapshot(self.store, out, self.key, "/work/main")
        self.assertEqual(list(self.store.iterdir()), [])

    def check(self, base_points, base_files=None, checkout_files=None):
        """main() comparing a working tree of [tick] with the base `self.key` names, the stored snapshot (if any)
        holding [tick] as well: (status, the sides built, the output). Git and the generator are stand-ins; a built
        base writes `base_points`. `base_files` are the base commit's files ({name: text}) and `checkout_files` those
        written into the checkout."""
        sides, base_files = [], base_files or {}
        checkout = self.tmp / "checkout"
        shutil.rmtree(checkout, ignore_errors=True)
        checkout.mkdir()
        for name, text in (checkout_files or {}).items():
            (checkout / name).write_text(text)

        def snapshot(root, out, target_dir, side):
            sides.append(side)
            out.mkdir()
            points = base_points if side == "base" else [point("tick")]
            (out / "points.json").write_text(json.dumps(points))
            (out / "api.rs").write_text(f"// generated from {root}/crates/horizon-client/src/game.rs\n")
            return 1.0, 2.0

        def git(*args, cwd=gate.ROOT):
            if args[0] == "rev-parse":
                return self.key["commit"]
            if args[0] == "ls-tree":
                return args[-1] if args[-1] in base_files else ""
            if args[0] == "show":
                return base_files[args[1].split(":", 1)[1]].strip()
            self.assertEqual(args[:2], ("worktree", "add"))
            Path(args[-2]).mkdir(parents=True)
            return ""

        # No real git: `git ls-files` lists nothing and the worktree removal does nothing.
        listed = gate.subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.object(gate, "snapshot", snapshot), mock.patch.object(gate, "git", git), \
                mock.patch.object(gate, "toolchain", lambda root: self.RUSTC), \
                mock.patch.object(gate, "ROOT", checkout), \
                mock.patch.object(gate.subprocess, "run", return_value=listed), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            status = gate.main(["--base", "HEAD", "--snapshots", str(self.store), "--target-dir", str(self.tmp)])
        return status, sides, out.getvalue()

    def test_a_hit_builds_only_the_working_tree(self):
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        self.stored()
        status, sides, out = self.check(base_points=[point("tick"), point("gone")])
        self.assertEqual((status, sides), (0, ["working tree"]), out)
        self.assertIn("Mod points base snapshot: hit; base 0 s (stored)", out)

    def test_a_miss_builds_the_base_and_compares_with_it(self):
        # The built base has a point the working tree lost: only a comparison with the built base fails.
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        for name, prepare, route in (
                ("nothing stored", lambda: None, "miss: none stored"),
                ("corrupt root", lambda: self.remarked(self.stored(), digest=False, root="/work/mail"),
                 "miss: incomplete (the marker does not match its digest)")):
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                prepare()
                status, sides, out = self.check(base_points=[point("tick"), point("gone")])
                self.assertEqual((status, sides), (1, ["base", "working tree"]), out)
                self.assertIn(f"Mod points base snapshot: {route}; base 3 s (compile 1 s, generator 2 s)", out)
                self.assertIn("Points removed and failing (1):", out)

    def test_a_toolchain_only_the_change_selects_never_hits(self):
        # The probe runs in the checkout, so a pin there could match a snapshot the base's compiler never made.
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        pin = '[toolchain]\nchannel = "1.99.0"\n'
        for name, base_files, checkout_files in (
                ("added", {}, {"rust-toolchain.toml": pin}),
                ("changed", {"rust-toolchain": "1.98.0"}, {"rust-toolchain": "1.99.0"}),
                ("removed", {"rust-toolchain.toml": pin}, {})):
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                self.stored()
                status, sides, out = self.check([point("tick"), point("gone")], base_files, checkout_files)
                self.assertEqual((status, sides), (1, ["base", "working tree"]), out)
                self.assertIn("Mod points base snapshot: miss: the change selects another toolchain;", out)
        with self.subTest("the same pin"):
            status, sides, out = self.check([point("tick")], {"rust-toolchain.toml": pin}, {"rust-toolchain.toml": pin})
            self.assertEqual((status, sides), (0, ["working tree"]), out)


class ToolchainFilesTests(unittest.TestCase):
    """selects_base_toolchain on a real repository: the checkout's toolchain files against a commit's."""

    def test_a_file_the_change_adds_edits_or_removes_selects_another_toolchain(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def commit(message):
                for args in (["add", "-A"], ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                                             "commit", "-q", "--allow-empty", "-m", message]):
                    subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
                return gate.git("rev-parse", "HEAD", cwd=root)

            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            (root / "lib.rs").write_text("")
            plain = commit("no pin")
            self.assertTrue(gate.selects_base_toolchain(plain, root))
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.0"\n')
            self.assertFalse(gate.selects_base_toolchain(plain, root), "untracked pin")
            pinned = commit("pin")
            self.assertTrue(gate.selects_base_toolchain(pinned, root))
            self.assertFalse(gate.selects_base_toolchain(plain, root), "committed pin")
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.0"\n')
            self.assertFalse(gate.selects_base_toolchain(pinned, root), "edited pin")
            (root / "rust-toolchain.toml").unlink()
            self.assertFalse(gate.selects_base_toolchain(pinned, root), "removed pin")


if __name__ == "__main__":
    unittest.main()
