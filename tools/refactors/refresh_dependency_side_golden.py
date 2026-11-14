#!/usr/bin/env python3
"""Refresh Side's intentional Wire break without compiling Rust.

Uses wire/mod.rs's layout_name/layout_mix and fieldless enum fold. First
reproduces the frozen two-variant baseline, then hashes the current declaration.
The normal golden renderer emits the Rust fixture; all other pins stay intact.
"""
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
from generate_wire_enum_goldens import render

MASK = (1 << 64) - 1

def name(value):
    result = 0xcbf29ce484222325
    for byte in value.encode():
        result = ((result ^ byte) * 0x100000001b3) & MASK
    return result

def mix(result, child):
    for byte in child.to_bytes(8, 'little'):
        result = ((result ^ byte) * 0x100000001b3) & MASK
    return result

def enum_hash(variants):
    result = name('Side')
    for variant in variants:
        result = mix(result, name(variant))
    return f"{mix(name('enum'), result):016x}"

source = ROOT / 'crates/horizon-mod-vocab/src/content/package_side.rs'
variants = re.findall(r'^    (\w+),$', source.read_text(), re.MULTILINE)
assert variants == ['Client', 'Both', 'Server'], variants
old, new = enum_hash(['Client', 'Both']), enum_hash(variants)
assert old == 'dd28eeec22f3e455'
path = ROOT / 'tools/fixtures/wire_enum_layouts.json'
manifest = json.loads(path.read_text())
rows = [row for row in manifest['enums'] if row['canonical_path'] == 'crate::content::package_side::Side']
assert len(rows) == 1 and rows[0]['hash'] in (old, new), rows
rows[0]['hash'] = new
path.write_text(json.dumps(manifest, indent=2) + '\n')
(ROOT / 'crates/horizon-mod-vocab/src/wire/fieldless_enum_goldens.rs').write_text(render(manifest))
print(f'Side: {old} -> {new}')
