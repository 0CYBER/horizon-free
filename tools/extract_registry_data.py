#!/usr/bin/env python3
"""Reproduce the private immutable block-registry storage extraction.

Run once on the pre-extraction tree. No file moves, public signature changes,
or interning policy are included. The lookup body is checked moved verbatim.
"""
from pathlib import Path
import hashlib
import re

ROOT = Path(__file__).resolve().parents[1]
path = ROOT / "crates/horizon-world/src/registry.rs"
text = path.read_text()
start = text.index("    pub fn id(&self, state: &BlockState)")
brace = text.index("{", start)
end = brace + 1
depth = 1
while depth:
    depth += (text[end] == "{") - (text[end] == "}")
    end += 1
method = text[start:end]
body = text[brace:end]
fields_start = text.index("    states: Vec<BlockState>,")
fields_end = text.index("    /// Saved palette entries", fields_start)
fields = text[fields_start:fields_end]
new_fields = re.sub(r"^    (states|ids|collided|defaults):", r"    pub(super) \1:", fields, flags=re.M)
data_method = method.replace("    pub fn id", "    pub(super) fn id", 1)
assert data_method[data_method.index("{"):] == body
new_file = ROOT / "crates/horizon-world/src/registry/data.rs"
assert not new_file.exists()
new_file.write_text(
    "//! Immutable imported registry data, separate from per-installation worker state.\n"
    "use super::{BlockState, BlockStateId, state_key};\n"
    "use std::collections::{BTreeMap, HashMap};\n\n"
    "pub(super) struct Data {\n" + new_fields + "}\n\n"
    "impl Data {\n" + data_method + "\n}\n"
)
text = text[:start] + "    pub fn id(&self, state: &BlockState) -> Option<BlockStateId> {\n        self.data.id(state)\n    }" + text[end:]
assert text.count(fields) == 1
text = text.replace(fields, "    data: std::sync::Arc<data::Data>,\n", 1)
text = text.replace("mod facts;", "mod data;\nmod facts;", 1)
old = """        let mut result = Self {
            mod_worker_resource: std::sync::OnceLock::new(),
            states: Vec::with_capacity(states.len()),
            ids: HashMap::with_capacity(states.len()),
            collided: BTreeMap::new(),
            defaults: BTreeMap::new(),
            saved_states: RwLock::new(FxHashMap::default()),
        };"""
new = """        let mut result = data::Data {
            states: Vec::with_capacity(states.len()),
            ids: HashMap::with_capacity(states.len()),
            collided: BTreeMap::new(),
            defaults: BTreeMap::new(),
        };"""
assert text.count(old) == 1
text = text.replace(old, new)
old = """        Ok(result)
    }
    pub fn len"""
new = """        Ok(Self {
            data: std::sync::Arc::new(result),
            mod_worker_resource: std::sync::OnceLock::new(),
            saved_states: RwLock::new(FxHashMap::default()),
        })
    }
    pub fn len"""
assert text.count(old) == 1
text = text.replace(old, new)
text = re.sub(r"\bself(\s*)\.(states|ids|collided|defaults)\b", r"self\1.data.\2", text)
path.write_text(text)
for name in ["json.rs", "snapshot.rs"]:
    p = path.parent / "registry" / name
    source = p.read_text()
    source = re.sub(r"\bregistry(\s*)\.(states|ids|collided|defaults)\b", r"registry\1.data.\2", source)
    if name == "snapshot.rs":
        old = """            states,
            ids: id_map,
            collided: collided_map,
            defaults: default_map,"""
        new = """            data: std::sync::Arc::new(super::data::Data {
                states,
                ids: id_map,
                collided: collided_map,
                defaults: default_map,
            }),"""
        assert source.count(old) == 1
        source = source.replace(old, new)
    p.write_text(source)
print("id lookup body moved verbatim:", hashlib.sha256(body.encode()).hexdigest())
print("No interning policy or public lifetime changes generated.")
