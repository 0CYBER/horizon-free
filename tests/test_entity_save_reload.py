import argparse
import os
from pathlib import Path
import tempfile
import unittest

from tools import entity_save_reload as harness


def binary_for(path):
    return harness.horizon_binary(argparse.Namespace(binary=path), Path('unused'))


class HorizonBinaryTests(unittest.TestCase):
    def test_relative_binary_is_resolved_against_the_callers_directory(self):
        # Each server runs with its world as the working directory, so the path must not stay relative.
        before = os.getcwd()
        with tempfile.TemporaryDirectory() as caller:
            os.chdir(caller)
            try:
                found = binary_for('target/fast/horizon-client')
                expected = Path(os.getcwd()) / 'target' / 'fast' / 'horizon-client'
            finally:
                os.chdir(before)
        self.assertTrue(found.is_absolute())
        self.assertEqual(found, expected)

    def test_absolute_binary_is_kept(self):
        binary = Path(os.path.abspath('horizon-client'))
        self.assertEqual(binary_for(str(binary)), binary)


if __name__ == '__main__':
    unittest.main()
