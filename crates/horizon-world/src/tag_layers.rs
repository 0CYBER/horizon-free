//! One tag's files over the resource stack, folded as NeoForge 26.3's
//! `TagLoader` folds them: each file's `values` are added, then its `remove`
//! entries (NeoForge's `TagFile.remove`) are removed, in pack order, and a
//! `replace` file drops what the lower files gave, removals included. The
//! readers that resolve tags from JSON build each tag from these layers.
use serde_json::Value;

/// A tag's entries over its layers, in order: an id or `#tag`, and whether
/// the entry removes (`TagLoader.EntryWithSource.remove`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagLayers(Vec<(String, bool)>);

/// A tag file entry's id: a string, or an object's `id` (`TagEntry.CODEC`).
pub fn entry_id(entry: &Value) -> Option<&str> {
    entry.as_str().or_else(|| entry.get("id")?.as_str())
}

impl TagLayers {
    /// The layers of one file.
    pub fn from_file(file: &Value) -> Self {
        let mut layers = Self::default();
        layers.push(file);
        layers
    }

    /// One more file, over the layers below it (conditions already
    /// checked): `replace` drops them, then its `values` and `remove` follow.
    /// Inside `values`, an entry of `{"id": ..., "remove": true}`
    /// (`TagLoader.EntryWithSource.remove`) removes like a `remove` entry,
    /// so one merged file can carry adds and removals in file order.
    pub fn push(&mut self, file: &Value) {
        if file.get("replace").and_then(Value::as_bool) == Some(true) {
            self.0.clear();
        }
        let push_entry = |entry: &Value, remove: bool, out: &mut Vec<(String, bool)>| {
            let Some(id) = entry_id(entry) else {
                return;
            };
            let removes = remove || (entry.get("remove").and_then(Value::as_bool) == Some(true));
            let (prefix, name) = id.strip_prefix('#').map_or(("", id), |name| ("#", name));
            let id = if name.contains(':') {
                format!("{prefix}{name}")
            } else {
                format!("{prefix}minecraft:{name}")
            };
            out.push((id, removes));
        };
        let mut next = Vec::new();
        if let Some(values) = file.get("values").and_then(Value::as_array) {
            values
                .iter()
                .for_each(|entry| push_entry(entry, false, &mut next));
        }
        if let Some(entries) = file.get("remove").and_then(Value::as_array) {
            entries
                .iter()
                .for_each(|entry| push_entry(entry, true, &mut next));
        }
        self.0.extend(next);
    }

    /// The `#tags` the entries name, adding or removing: what building
    /// the tag needs built first.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter_map(|(entry, _)| entry.strip_prefix('#'))
    }

    /// The tag's members (`TagLoader.tryBuildTag`): each entry in order adds
    /// to or removes from an insertion-ordered set; a `#tag` adds or removes
    /// every member `members` gives for it. A removal never fails the tag.
    pub fn build(&self, mut members: impl FnMut(&str) -> Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (entry, remove) in &self.0 {
            let ids = match entry.strip_prefix('#') {
                Some(tag) => members(tag),
                None => vec![entry.clone()],
            };
            for id in ids {
                let at = out.iter().position(|e| *e == id);
                match (at, remove) {
                    (Some(i), true) => {
                        out.remove(i);
                    }
                    (None, false) => out.push(id),
                    _ => {}
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn one_file_adds_and_removes_the_same_entry_and_remove_objects_apply_in_order() {
        let things = TagLayers::from_file(&json!({
            "values": [
                "minecraft:stone",
                {"id": "minecraft:stone", "remove": true},
                "minecraft:dirt",
                {"id": "minecraft:gravel", "remove": true, "required": false}
            ]
        }));
        assert_eq!(things.build(|_| Vec::new()), ["minecraft:dirt"]);
    }

    #[test]
    fn removals_apply_in_layer_order_and_replace_drops_them() {
        let mut things =
            TagLayers::from_file(&json!({"values": ["minecraft:stone", "minecraft:dirt"]}));
        things.push(&json!({"values": [], "remove": ["minecraft:dirt", "#test:other"]}));
        let other =
            TagLayers::from_file(&json!({"values": [{"id": "minecraft:sand", "required": false}]}));
        let tags = HashMap::from([("test:other", other.clone())]);
        let members = |tag: &str| {
            tags.get(tag)
                .map(|t| t.build(|_| Vec::new()))
                .unwrap_or_default()
        };
        things.push(&json!({"values": ["minecraft:sand", "minecraft:gravel"]}));
        // The sand the removal of `#test:other` took out is added back above it.
        assert_eq!(
            things.build(members),
            ["minecraft:stone", "minecraft:sand", "minecraft:gravel"]
        );
        let mut removed = things.clone();
        removed.push(&json!({"values": [], "remove": ["#test:other", "minecraft:missing"]}));
        assert_eq!(
            removed.build(members),
            ["minecraft:stone", "minecraft:gravel"]
        );
        assert_eq!(
            removed.tags().collect::<Vec<_>>(),
            ["test:other", "test:other"]
        );
        things.push(&json!({"replace": true, "values": ["minecraft:dirt"]}));
        assert_eq!(things.build(members), ["minecraft:dirt"]);
    }
}
