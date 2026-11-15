"""The ignored-tests matrix names every leg of a split package, and only those."""
import unittest

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


if __name__ == "__main__":
    unittest.main()
