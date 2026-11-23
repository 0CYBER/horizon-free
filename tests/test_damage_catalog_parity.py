import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import parity

TOOLS = Path(__file__).resolve().parents[1] / "tools"


def _load_harness():
    spec = importlib.util.spec_from_file_location(
        "_damage_catalog_parity_harness", TOOLS / "test_damage_catalog_parity.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.path.append(str(TOOLS))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(TOOLS))
    return module


harness = _load_harness()


CLIENT_TESTS = [
    "game::network::lan::tests::datapack_damage_world_open_reuses_the_no_pack_preload",
    "game::network::lan::tests::datapack_damage_world_open_replaces_preload_atomically",
    "game::network::lan::tests::datapack_damage_unrelated_packs_do_not_load_sync_data_at_open",
    "game::network::lan::tests::datapack_damage_lazy_wire_snapshot_survives_reload_before_first_hub",
    "game::network::lan::tests::datapack_damage_lazy_wire_load_caches_after_all_packs_are_removed",
    "game::network::lan::tests::datapack_damage_registry_reload_keeps_wire_ids_and_reloads_tags_before_future_join",
]

CLIENT_LIB_TESTS = [
    "health::datapack_tests::datapack_damage_new_world_drops_a_removed_resource_root_definition",
    "health::datapack_tests::datapack_damage_initial_snapshot_uses_the_prepared_worlds_features",
    "health::datapack_tests::datapack_damage_uses_custom_identity_armor_fire_and_reloaded_tags",
    "landing::tests::datapack_damage_landing_uses_world_armor_tags_and_exhaustion",
]


def _cargo_ok(tests):
    return "".join(f"test {name} ... ok\n" for name in tests)


class DamageCatalogSnapshotTests(unittest.TestCase):
    def test_snapshot_tests_runs_client_and_client_lib_sequentially(self):
        commands = []

        def fake_run(cmd, **_kwargs):
            commands.append(cmd)
            if "horizon-client" in cmd:
                stdout = _cargo_ok(CLIENT_TESTS)
            else:
                stdout = _cargo_ok(CLIENT_LIB_TESTS)
            return subprocess.CompletedProcess(cmd, 0, stdout=stdout, stderr="")

        with tempfile.TemporaryDirectory() as tmp:
            build = Path(tmp)
            with mock.patch.object(harness.subprocess, "run", side_effect=fake_run):
                passed = harness.snapshot_tests(build)
            log = (build / "snapshot-tests.log").read_text()

        self.assertEqual(commands, [
            ["cargo", "test", "--locked", "-p", "horizon-client",
             "--lib", "datapack_damage_", "--", "--include-ignored"],
            ["cargo", "test", "--locked", "-p", "horizon-client-lib",
             "--lib", "datapack_damage_", "--", "--include-ignored"],
        ])
        self.assertEqual(passed, CLIENT_TESTS + CLIENT_LIB_TESTS)
        self.assertIn("=== horizon-client (exit 0) ===", log)
        self.assertIn("=== horizon-client-lib (exit 0) ===", log)
        selections = parity.cargo_tests(commands)
        self.assertEqual(parity.prebuild_commands([], {}, selections), [
            ["cargo", "test", "--locked", "--no-run", "-p", "horizon-client", "--lib"],
            ["cargo", "test", "--locked", "--no-run", "-p", "horizon-client-lib", "--lib"],
        ])

    def test_snapshot_tests_logs_failure_cause_and_stops(self):
        commands = []

        def fake_run(cmd, **_kwargs):
            commands.append(cmd)
            return subprocess.CompletedProcess(cmd, -9, stdout="", stderr="killed by OOM\n")

        buf = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            build = Path(tmp)
            with mock.patch.object(harness.subprocess, "run", side_effect=fake_run), \
                 contextlib.redirect_stdout(buf):
                with self.assertRaises(subprocess.CalledProcessError) as ctx:
                    harness.snapshot_tests(build)
            log = (build / "snapshot-tests.log").read_text()

        self.assertEqual(ctx.exception.returncode, -9)
        self.assertEqual(len(commands), 1)
        self.assertIn("horizon-client", buf.getvalue())
        self.assertIn("signal 9", buf.getvalue())
        self.assertIn("killed by OOM", log)


if __name__ == "__main__":
    unittest.main()
