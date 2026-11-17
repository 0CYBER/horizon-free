#!/usr/bin/env python3
"""Compares two state digests step by step and names the first differing step per group: A/digest.txt B/digest.txt."""
import sys

GROUPS = ['entities', 'randoms', 'block_entities', 'level_random', 'counters', 'scheduled']


def read(path):
    running, frozen_moves, last_random = [], 0, None
    for line in open(path):
        if not line.startswith('tick='):
            continue
        fields = dict(part.split('=', 1) for part in line.split())
        if fields['run'] == '1':
            running.append(fields)
        elif last_random is not None and fields['level_random'] != last_random:
            frozen_moves += 1
        last_random = fields['level_random']
    return running, frozen_moves


def main():
    (a, fa), (b, fb) = read(sys.argv[1]), read(sys.argv[2])
    ticks = range(min(len(a), len(b)))
    missing = [] if len(a) == len(b) else [len(ticks)]
    print(f'running ticks: {len(a)} vs {len(b)}, game time {a[0]["tick"]}..{a[-1]["tick"]} vs {b[0]["tick"]}..{b[-1]["tick"]}')
    print(f'frozen ticks that moved the level random: {fa} vs {fb}')
    same = True
    for group in GROUPS:
        diff = [t for t in ticks if a[t][group] != b[t][group]]
        if diff:
            same = False
            t = diff[0]
            print(f'{group:15} DIFFERS on {len(diff)} steps, first step {t + 1} (tick {a[t]["tick"]}): {a[t][group]} vs {b[t][group]}')
        else:
            print(f'{group:15} identical')
    print('IDENTICAL' if same and not missing else 'DIFFERENT')
    return 0 if same and not missing else 1


if __name__ == '__main__':
    sys.exit(main())
