from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_geometry_parity as geometry
from tools import test_collision_parity as collision


class GeometryRunnerTests(unittest.TestCase):
    def test_bad_artifacts_remove_old_success_reports(self):
        for module, name in [(geometry, "rust-geometry-parity"), (collision, "rust-collision-parity")]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                local = Path(temp)
                report = local / name / "report.json"
                report.parent.mkdir()
                report.write_text('{"previous_success":true}')
                with mock.patch.object(module.lab, "LOCAL", local), mock.patch("sys.argv", ["parity"]), \
                     mock.patch.object(module.test_rust_parity, "verify_artifacts", side_effect=ValueError("corrupt artifact")):
                    with self.assertRaisesRegex(ValueError, "corrupt artifact"):
                        module.main()
                self.assertFalse(report.exists())

    def test_collision_counts_reject_missing_partial_and_inconsistent_coverage(self):
        for output in ["", "HORIZON_COLLISION_RESULT {}", 'HORIZON_COLLISION_RESULT {"registry_states":35723,"cached_states":1,"uncached_states":1,"queries":6,"unique_shapes":1}']:
            with self.subTest(output=output), self.assertRaises((AssertionError, KeyError)):
                collision.counts(output)

    def test_geometry_inputs_are_repeatable_and_include_ieee_boundaries(self):
        commands = list(geometry.corpus())
        self.assertEqual(commands, list(geometry.corpus()))
        joined = "\n".join(commands)
        for value in ["8000000000000000", "7ff0000000000000", "fff0000000000000", "7ff8000000000000", "0000000000000001"]:
            self.assertIn(value, joined)
        operations = {line.split()[0] for line in commands}
        self.assertEqual(operations, {"vec", "box", "block-box", "grid", "query", "many", "motion", "step-motion", "trig-table", "rotation", "traverse", "join", "make-box", "optimize", "join-optimized", "block-iterate", "box-iterate", "sweep", "floor-height"})
        self.assertEqual({int(line.split()[-1]) for line in commands if line.startswith("join-optimized ")}, set(range(16)))


if __name__ == "__main__":
    unittest.main()
