#!/usr/bin/env python3
"""Regression for tools/redstone_diff.py::diff_frames (finding t01).

Pure Python: imports redstone_diff (argparse/json/random/etc. + lab), no game, Java,
Cargo, network or vanilla data is started.
"""
import unittest

from tools import redstone_diff as rd


def frame(tick, **blocks):
    return {'tick': tick, 'blocks': dict(blocks)}


class DiffFramesTest(unittest.TestCase):
    def test_truncated_second_timeline_is_a_difference(self):
        full = [frame(0, pos='minecraft:stone'), frame(1, pos='minecraft:redstone_block')]
        short = [full[0]]
        self.assertEqual(
            rd.diff_frames(full, short),
            (1, [('pos', 'minecraft:redstone_block', rd.MISSING_TICK)]),
        )

    def test_truncated_first_timeline_is_a_difference(self):
        short = [frame(0, pos='minecraft:stone')]
        full = [short[0], frame(1, pos='minecraft:redstone_block')]
        self.assertEqual(
            rd.diff_frames(short, full),
            (1, [('pos', rd.MISSING_TICK, 'minecraft:redstone_block')]),
        )

    def test_equal_timelines_are_identical(self):
        a = [frame(0, pos='minecraft:stone'), frame(1, pos='minecraft:redstone_block')]
        b = [dict(f, blocks=dict(f['blocks'])) for f in a]
        self.assertIsNone(rd.diff_frames(a, b))

    def test_both_empty_are_identical(self):
        self.assertIsNone(rd.diff_frames([], []))

    def test_earlier_prefix_difference_wins_over_missing_tail(self):
        a = [frame(0, pos='minecraft:stone'), frame(1, pos='minecraft:redstone_block')]
        b = [frame(0, pos='minecraft:air')]  # tick 0 differs and tick 1 is absent
        self.assertEqual(
            rd.diff_frames(a, b),
            (0, [('pos', 'minecraft:stone', 'minecraft:air')]),
        )

    def test_equal_length_block_difference_unchanged(self):
        a = [frame(0, pos='minecraft:stone')]
        b = [frame(0, pos='minecraft:air')]
        self.assertEqual(
            rd.diff_frames(a, b),
            (0, [('pos', 'minecraft:stone', 'minecraft:air')]),
        )

    def test_missing_tail_reports_every_block_of_the_first_extra_tick(self):
        a = [frame(0, pos='minecraft:stone'), frame(1, a='minecraft:stone', b='minecraft:air')]
        b = [a[0]]
        tick, rows = rd.diff_frames(a, b)
        self.assertEqual(tick, 1)
        self.assertEqual(rows, [('a', 'minecraft:stone', rd.MISSING_TICK),
                                ('b', 'minecraft:air', rd.MISSING_TICK)])


if __name__ == '__main__':
    unittest.main()
