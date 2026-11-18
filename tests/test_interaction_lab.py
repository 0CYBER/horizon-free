import contextlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import interaction_lab


class CargoExampleTests(unittest.TestCase):
    def test_client_probe_uses_the_reusable_library_package(self):
        with mock.patch.object(interaction_lab.subprocess, 'run') as run:
            interaction_lab.cargo_example('horizon-client', 'menu_fixture', Path('cases.txt'), profile='fast')
        run.assert_called_once_with(
            ['cargo', 'run', '-q', '-j3', '--profile', 'fast', '-p', 'horizon-client-lib',
             '--example', 'client_probe', '--', 'menu_fixture', 'cases.txt'],
            check=True, cwd=interaction_lab.lab.ROOT)

    def test_other_probe_packages_are_unchanged(self):
        with mock.patch.object(interaction_lab.subprocess, 'run') as run:
            interaction_lab.cargo_example('horizon-world', 'loot_probe', 42)
        run.assert_called_once_with(
            ['cargo', 'run', '-q', '-j3', '-p', 'horizon-world', '--example', 'world_probe', '--', 'loot_probe', '42'],
            check=True, cwd=interaction_lab.lab.ROOT)


class CompareTests(unittest.TestCase):
    def test_empty_cases_reject_unexpected_output(self):
        for native_text, rust_text, expected in (
            ('', '', 0),
            ('unexpected\n', '', 1),
            ('', 'unexpected\n', 1),
            ('unexpected\n', 'unexpected\n', 1),
        ):
            with self.subTest(native=native_text, rust=rust_text), tempfile.TemporaryDirectory() as d:
                native = Path(d) / 'native.txt'
                rust = Path(d) / 'rust.txt'
                native.write_text(native_text)
                rust.write_text(rust_text)
                with contextlib.redirect_stdout(io.StringIO()):
                    total = interaction_lab.compare([], native, rust, key=lambda case, a: case)
                self.assertEqual(total, expected)

    def test_truncated_rust_output_is_a_failure(self):
        cases = [{'i': i} for i in range(5)]
        with tempfile.TemporaryDirectory() as d:
            native = Path(d) / 'native.txt'
            rust = Path(d) / 'rust.txt'
            native.write_text(''.join(f'line{i}\n' for i in range(5)))
            rust.write_text(''.join(f'line{i}\n' for i in range(2)))
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                total = interaction_lab.compare(cases, native, rust, key=lambda case, a: case['i'])
        self.assertGreater(total, 0)
        self.assertIn('line counts differ', out.getvalue())
        self.assertNotIn('5/5 cases match', out.getvalue())

    def test_equal_length_outputs_compare_normally(self):
        cases = [{'i': i} for i in range(5)]
        with tempfile.TemporaryDirectory() as d:
            native = Path(d) / 'native.txt'
            rust = Path(d) / 'rust.txt'
            native.write_text(''.join(f'line{i}\n' for i in range(5)))
            rust.write_text(''.join(f'line{i}\n' if i != 3 else 'changed\n' for i in range(5)))
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                total = interaction_lab.compare(cases, native, rust, key=lambda case, a: case['i'])
        self.assertEqual(total, 1)
        self.assertIn('4/5 cases match', out.getvalue())
