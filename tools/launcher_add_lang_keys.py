#!/usr/bin/env python3
"""Update source-English keys in every launcher language catalog, preserving translations."""

import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("keys", nargs="+", help="New source-English strings used by tr/trf")
parser.add_argument("--remove", action="append", default=[], help="Obsolete source-English key")
args = parser.parse_args()

for path in sorted((Path(__file__).resolve().parents[1] / "launcher/lang").glob("*.json")):
    source = path.read_text(encoding="utf-8")
    table = json.loads(source)
    for key in args.remove:
        table.pop(key, None)
    for key in args.keys:
        table.setdefault(key, key)
    updated = json.dumps(table, ensure_ascii=False, indent=2) + "\n"
    if updated != source:
        path.write_text(updated, encoding="utf-8")
