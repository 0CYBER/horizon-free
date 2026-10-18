import unittest

from tools.server_bench import work


def run(server, bots, sent, new, waited=0.0, behaviour="mixed"):
    return {"server": server, "behaviour": behaviour, "bots": bots, "chunks_per_second": sent,
            "unique_per_second": new, "waited_ticks_per_second": waited}


class ServerBenchWorkTests(unittest.TestCase):
    def test_shares_of_the_busiest_server(self):
        [row] = work([run("horizon", 10, 98.2, 36.5), run("vanilla", 10, 24.3, 19.1, 12.0)])
        self.assertFalse(row["equal"])
        self.assertEqual(row["servers"]["horizon"]["chunks_per_second"], {"value": 98.2, "share": 1.0})
        self.assertEqual(row["servers"]["vanilla"]["chunks_per_second"]["share"], 0.247)
        self.assertEqual(row["servers"]["vanilla"]["waited_ticks_per_second"], 12.0)

    def test_equal_within_ten_percent_or_a_chunk(self):
        [row] = work([run("horizon", 25, 236, 28.9), run("vanilla", 25, 252, 28.9)])
        self.assertTrue(row["equal"])
        # One bot: 0.3 against 1.1 new chunks a second is within a chunk.
        [row] = work([run("horizon", 1, 9.9, 0.3), run("vanilla", 1, 9.9, 1.1)])
        self.assertTrue(row["equal"])

    def test_only_counts_run_on_several_servers(self):
        failed = dict(run("vanilla", 50, 0, 0), error="watchdog")
        self.assertEqual(work([run("horizon", 50, 450, 10), failed, run("horizon", 1, 9, 0.3)]), [])


if __name__ == "__main__":
    unittest.main()
