//! One tag's files over the resource stack, folded as NeoForge 26.3's
//! `TagLoader` folds them: each file's `values` are added, then its `remove`
//! entries (NeoForge's `TagFile.remove`) are removed, in pack order, and a
//! `replace` file drops what the lower files gave, removals included. The
//! readers that resolve tags from JSON build each tag from these layers.
use serde_json::Value;
use std::{borrow::Cow, collections::HashMap};

mod resolve;

#[cfg(test)]
pub(crate) mod work_budget {
    use std::cell::Cell;

    thread_local! { static REMAINING: Cell<Option<usize>> = const { Cell::new(None) }; }

    pub(crate) fn check_step() {
        REMAINING.with(|budget| {
            if let Some(remaining) = budget.get() {
                budget.set(Some(
                    remaining
                        .checked_sub(1)
                        .expect("tag traversal exceeded its work budget"),
                ));
            }
        });
    }

    pub(crate) fn with_limit<T>(limit: usize, run: impl FnOnce() -> T) -> T {
        REMAINING.with(|budget| budget.set(Some(limit)));
        let result = run();
        REMAINING.with(|budget| budget.set(None));
        result
    }
}

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
        self.build_ordered(|tag| members(tag).into_iter().map(Cow::Owned))
    }

    fn build_ordered<'a, I: IntoIterator<Item = Cow<'a, str>>>(
        &'a self,
        mut members: impl FnMut(&str) -> I,
    ) -> Vec<String> {
        let mut positions = HashMap::new();
        let mut next_position = 0;
        for (entry, remove) in &self.0 {
            let mut apply = |id: Cow<'a, str>| {
                if *remove {
                    positions.remove(id.as_ref());
                } else if let std::collections::hash_map::Entry::Vacant(slot) = positions.entry(id)
                {
                    slot.insert(next_position);
                    next_position += 1;
                }
            };
            if let Some(tag) = entry.strip_prefix('#') {
                members(tag).into_iter().for_each(apply);
            } else {
                apply(Cow::Borrowed(entry));
            }
        }
        // Removed positions stay empty. Re-adding a value gives it a new position.
        let mut ordered = vec![None; next_position];
        for (id, position) in positions {
            ordered[position] = Some(id.into_owned());
        }
        ordered.into_iter().flatten().collect()
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
