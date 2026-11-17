"""Regression for parity_shard.label()'s output-directory name when PARITY_SHARD is empty or unset."""
import os
import unittest
from unittest import mock

from tools import parity_shard


class LabelTest(unittest.TestCase):
    def test_empty_shard_spec_labels_all(self):
        # PARITY_SHARD="" is a common way to leave sharding disabled: shard() runs the whole corpus,
        # so label() must give the same full-run name instead of a falsy, colliding directory name.
        with mock.patch.dict(os.environ, {"PARITY_SHARD": ""}):
            self.assertEqual(parity_shard.shard(["a", "b", "c"]), ["a", "b", "c"])
            self.assertEqual(parity_shard.label(), "all")

    def test_unset_shard_spec_labels_all(self):
        with mock.patch.dict(os.environ):
            os.environ.pop("PARITY_SHARD", None)
            self.assertEqual(parity_shard.label(), "all")

    def test_named_shard_label_unchanged(self):
        with mock.patch.dict(os.environ, {"PARITY_SHARD": "2/4"}):
            self.assertEqual(parity_shard.label(), "2-4")


if __name__ == "__main__":
    unittest.main()
