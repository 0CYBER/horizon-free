import unittest

import re

from tools.server_bench import DONE, TICK_AVERAGE, TICK_PERCENTILES, work


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


class ServerBenchLogTests(unittest.TestCase):
    def test_every_server_says_it_is_up_the_same_way(self):
        for line in ['[05:16:10] [Server thread/INFO]: Done (9.762s)! For help, type "help"',
                     '[02:53:58 INFO]: Done (29.649s)! For help, type "help"']:
            self.assertTrue(re.search(DONE, line), line)
        self.assertFalse(re.search(DONE, '[02:53:58 INFO]: Preparing spawn area: Done (12%)'))

    def test_paper_answers_tick_query_as_vanilla_does(self):
        # Paper 26.3 build 205, from a smoke run of this tool.
        self.assertEqual(TICK_AVERAGE.search("Average time per tick: 14.4ms (Target: 50.0ms)").group(1), "14.4")
        self.assertEqual(TICK_PERCENTILES.search(
            "[02:54:10 INFO]: System chat: Percentiles: P50: 1.0ms P95: 34.6ms P99: 764.4ms. Sample: 100").groups(),
            ("1.0", "34.6", "764.4"))
