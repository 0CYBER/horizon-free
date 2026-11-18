"""Pure-Python regressions for the environment render parity harness."""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import test_environment_render_parity as env
from tools import test_rust_parity as base


class FieldMismatchTests(unittest.TestCase):
    def test_list_length_and_value_comparison(self):
        tolerance = 1e-4
        # Equal-length differing values are a mismatch.
        self.assertTrue(env.field_mismatch([1.0, 2.0], [1.0, 3.0], tolerance))
        # Different-length prefix-equal lists are a mismatch: zip() must not
        # silently drop the missing tail.
        self.assertTrue(env.field_mismatch([1.0, 2.0, 3.0, 4.0], [1.0, 2.0], tolerance))
        # Equal lists are not a mismatch.
        self.assertFalse(env.field_mismatch([1.0, 2.0, 3.0, 4.0], [1.0, 2.0, 3.0, 4.0], tolerance))
        # Scalars still work.
        self.assertFalse(env.field_mismatch(1.0, 1.00001, tolerance))


class MainComparisonTests(unittest.TestCase):
    def _run_main(self, vanilla_records, rust_records):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            build = root / 'rust-environment-parity'
            (root / 'resources' / 'data/minecraft/worldgen/biome').mkdir(parents=True)

            def fake_run(command, **kwargs):
                text = ' '.join(map(str, command))
                if 'VanillaEnvironmentOracle' in text:
                    (build / 'vanilla.jsonl').write_text(
                        '\n'.join(json.dumps(r) for r in vanilla_records) + '\n')
                elif 'environment_fixture' in text:
                    (build / 'rust.jsonl').write_text(
                        '\n'.join(json.dumps(r) for r in rust_records) + '\n')
                return type('R', (), {'returncode': 0})()

            artifacts = ({'minecraft': {'java_major': 25}},
                         {'server': Path('srv.jar')}, [Path('dep.jar')])
            with mock.patch.object(base, 'verify_artifacts', return_value=artifacts), \
                 mock.patch.object(env.lab, 'LOCAL', root), \
                 mock.patch.object(env.lab, 'RESOURCES', root / 'resources'), \
                 mock.patch.object(env.subprocess, 'run', side_effect=fake_run), \
                 mock.patch.object(sys, 'argv', ['prog']):
                output = io.StringIO()
                exit_code = 0
                try:
                    with contextlib.redirect_stdout(output):
                        env.main()
                except SystemExit as exc:
                    exit_code = exc.code if isinstance(exc.code, int) else 1
            return exit_code, output.getvalue()

    def test_truncated_rust_list_fails_the_harness(self):
        case = 'overworld 0 0 0 0 plains'
        code, output = self._run_main(
            [{'case': case, 'sky_light': [1.0, 2.0, 3.0, 4.0]}],
            [{'case': case, 'sky_light': [1.0, 2.0]}])
        self.assertNotEqual(code, 0)
        self.assertIn('MISMATCH sky_light', output)

    def test_equal_lists_stay_green(self):
        case = 'overworld 0 0 0 0 plains'
        code, output = self._run_main(
            [{'case': case, 'sky_light': [1.0, 2.0, 3.0, 4.0]}],
            [{'case': case, 'sky_light': [1.0, 2.0, 3.0, 4.0]}])
        self.assertEqual(code, 0)
        self.assertIn('environment parity ok', output)


if __name__ == '__main__':
    unittest.main()
