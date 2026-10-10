#!/usr/bin/env python3
"""Group the mismatches of tools/test_command_parse_parity.py by cause.

usage: command_parse_summary.py MISMATCHES.jsonl [ROOT]
"""
import collections
import json
import sys
from pathlib import Path


def describe(side, kind):
    if kind == 'parse':
        if side.get('ok'):
            return 'OK'
        return f"ERR {side.get('error')!r}@{'c' if side.get('cursor') else '0'}"
    return f"SUGG[{len(side['list'])}]"


path = Path(sys.argv[1])
only = sys.argv[2] if len(sys.argv) > 2 else None
groups = collections.Counter()
examples = {}
for line in path.open(encoding='utf-8'):
    m = json.loads(line)
    root = m['in'].split(' ')[0]
    if only and root != only:
        continue
    key = (m['kind'], describe(m['vanilla'], m['kind']), describe(m['rust'], m['kind']))
    if m['kind'] == 'parse':
        key = key[:1] + (key[1][:90], key[2][:90])
    groups[key] += 1
    examples.setdefault(key, m)
for key, count in sorted(groups.items(), key=lambda kv: -kv[1])[:int(sys.argv[3]) if len(sys.argv) > 3 else 40]:
    ex = examples[key]
    print(f'{count:6} {key[0]} vanilla={key[1]} rust={key[2]}\n         e.g. {ex["in"]!r}')
