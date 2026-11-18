"""The ignored-tests matrix names every leg of a split package, and only those."""
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.ci import ignored_shards


class ShardMatrix(unittest.TestCase):
    def test_a_split_package_becomes_one_entry_per_leg(self):
        self.assertEqual(
            ignored_shards.expand(["horizon-client", "horizon-world"]),
            ["horizon-client#1of2", "horizon-client#2of2", "horizon-world"],
        )

    def test_every_shard_is_covered_by_the_full_matrix(self):
        entries = ignored_shards.expand(ignored_shards.SHARDS)
        for package in ignored_shards.SHARDS:
            legs = ignored_shards.LEGS.get(package, 1)
            owned = [entry for entry in entries if entry.split("#")[0] == package]
            self.assertEqual(len(owned), legs)

    def test_a_package_without_legs_keeps_its_plain_name(self):
        self.assertEqual(ignored_shards.expand(["horizon-items"]), ["horizon-items"])


class BuildDependencySelection(unittest.TestCase):
    def test_a_change_to_a_build_only_helper_selects_the_shard_that_uses_it(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name, manifest in (
                ("shard", '[package]\nname = "shard"\nversion = "0.0.0"\n\n[build-dependencies]\n'
                          'helper = { path = "../helper" }\n'),
                ("helper", '[package]\nname = "helper"\nversion = "0.0.0"\n'),
            ):
                crate = root / "crates" / name
                crate.mkdir(parents=True)
                (crate / "Cargo.toml").write_text(manifest)
            with mock.patch.object(ignored_shards, "ROOT", root), \
                    mock.patch.object(ignored_shards, "SHARDS", ["shard"]):
                dependencies = ignored_shards.workspace_dependencies()
                selected = ignored_shards.select(["crates/helper/src/lib.rs"], dependencies)
        self.assertEqual(dependencies["shard"], {"helper"})
        self.assertEqual(selected, ["shard"])


if __name__ == "__main__":
    unittest.main()
