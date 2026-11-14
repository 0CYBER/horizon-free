#!/usr/bin/env python3
"""Extract the shared data-map file merger without moving host-answer bodies."""
from pathlib import Path

root = Path('crates/horizon-client/src/game/modding')
p = root / 'native_data_maps.rs'
s = p.read_text()
a = s.index('/// `minecraft:` for an id written without a namespace.')
b = s.index('impl DataMaps {', a)
block = s[a:b]
for name in ['apply_file', 'apply_files']:
    block = block.replace(f'fn {name}(', f'pub(super) fn {name}(')
destination = root / 'data_map_files.rs'
assert not destination.exists()
destination.write_text('//! Shared pack-order data-map file merging.\nuse serde_json::Value;\nuse std::collections::{BTreeMap, HashMap, HashSet};\n\n' + block)
s = s[:a] + s[b:]
s = s.replace('use super::data_map_json::to_tag;', 'use super::data_map_json::to_tag;\nuse super::data_map_files::apply_files;\n#[cfg(test)]\nuse super::data_map_files::apply_file;')
p.write_text(s)
p = root / 'neoforge_mob_maps.rs'
s = p.read_text().replace('use super::native_data_maps::merge_builtin;', 'use super::data_map_files::merge_builtin;')
p.write_text(s)
