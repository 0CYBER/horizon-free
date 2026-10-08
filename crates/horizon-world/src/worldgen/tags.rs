//! Version-pinned tag codecs, resource-stack merging and dependency resolution.
use super::{DecodeError, Object, optional, required};
use horizon_core::random::java_string_hash;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntry {
    pub id: String,
    pub tag: bool,
    pub required: bool,
}
impl TagEntry {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        let (name, mandatory) = if raw.get().starts_with('"') {
            (serde_json::from_str::<String>(raw.get())?, true)
        } else {
            let object: Object = serde_json::from_str(raw.get())?;
            (
                serde_json::from_str(required(&object, "id")?.get())?,
                optional(&object, "required")
                    .map(|v| serde_json::from_str(v.get()))
                    .transpose()?
                    .unwrap_or(true),
            )
        };
        let (tag, name) = name
            .strip_prefix('#')
            .map_or((false, name.as_str()), |s| (true, s));
        Ok(Self {
            id: identifier(name)?,
            tag,
            required: mandatory,
        })
    }
    pub fn display(&self) -> String {
        format!(
            "{}{}{}",
            if self.tag { "#" } else { "" },
            self.id,
            if self.required { "" } else { "?" }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagFile {
    pub entries: Vec<TagEntry>,
    pub replace: bool,
}
impl TagFile {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let values: Vec<Box<RawValue>> = serde_json::from_str(required(&object, "values")?.get())?;
        let entries = values
            .iter()
            .map(|v| TagEntry::from_json(v.get()))
            .collect::<Result<_, _>>()?;
        let replace = optional(&object, "replace")
            .map(|v| serde_json::from_str(v.get()))
            .transpose()?
            .unwrap_or(false);
        Ok(Self { entries, replace })
    }
}
// Identifier.CODEC permits the empty path as well as an omitted namespace.
pub(crate) fn identifier(name: &str) -> Result<String, DecodeError> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid) || !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(DecodeError::Registry(format!(
            "invalid tag identifier: {name}"
        )));
    }
    Ok(format!("{namespace}:{path}"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntryWithSource {
    pub entry: TagEntry,
    pub source: String,
}
#[derive(Clone, Debug, Default)]
pub struct TagDefinitions {
    entries: BTreeMap<String, Vec<TagEntryWithSource>>,
    insertion_order: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct BuiltTags<T> {
    pub tags: BTreeMap<String, Vec<T>>,
    pub missing: BTreeMap<String, Vec<TagEntryWithSource>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagBuildError {
    TooManyTags,
    DependencyDepth,
    HashTreeBucket,
}
impl std::fmt::Display for TagBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooManyTags => "tag definitions exceed 8192 entries",
            Self::DependencyDepth => {
                "tag dependency depth exceeds 256 or cyclic reachability does not terminate"
            }
            Self::HashTreeBucket => "tag identifiers require an unsupported Java hash tree bucket",
        })
    }
}
impl std::error::Error for TagBuildError {}
impl TagDefinitions {
    /// Apply resources in their pack priority order. A successfully decoded
    /// replace file clears earlier entries; a rejected file changes nothing.
    pub fn add(&mut self, name: &str, source: &str, file: TagFile) -> Result<(), DecodeError> {
        let name = identifier(name)?;
        if !self.entries.contains_key(&name) {
            self.insertion_order.push(name.clone());
        }
        let entries = self.entries.entry(name).or_default();
        if file.replace {
            entries.clear();
        }
        entries.extend(file.entries.into_iter().map(|entry| TagEntryWithSource {
            entry,
            source: source.into(),
        }));
        Ok(())
    }
    /// A valid JSON resource with a codec error still creates an empty tag in
    /// vanilla's resource loader; existing entries are retained on that error.
    pub fn apply_json(&mut self, name: &str, source: &str, json: &str) -> Result<(), DecodeError> {
        let _: Box<RawValue> = serde_json::from_str(json)?;
        let name = identifier(name)?;
        if !self.entries.contains_key(&name) {
            self.insertion_order.push(name.clone());
            self.entries.insert(name.clone(), Vec::new());
        }
        self.add(&name, source, TagFile::from_json(json)?)
    }
    pub fn definitions(&self) -> &BTreeMap<String, Vec<TagEntryWithSource>> {
        &self.entries
    }
    /// Missing required references invalidate their own tags, not unrelated tags.
    /// Element lookup receives the entry's required flag, as in vanilla.
    pub fn build<T: Clone + Ord>(
        &self,
        mut lookup: impl FnMut(&str, bool) -> Option<T>,
    ) -> Result<BuiltTags<T>, TagBuildError> {
        if self.entries.len() > 8192 {
            return Err(TagBuildError::TooManyTags);
        }
        // DependencySorter owns a fresh default-sized HashMap. Required edges
        // are inserted for every tag before any optional edge is considered.
        // The resource loader creates keys with HashMap.computeIfAbsent,
        // which inserts at the head of a bucket. DependencySorter then copies
        // that map with put (tail insertion), retaining the reversed order of
        // colliding resource keys. Resizing preserves each split chain's order.
        let order = java_bucket_order(self.insertion_order.iter().rev().map(String::as_str), 16)?;
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for mandatory in [true, false] {
            for &name in &order {
                for entry in &self.entries[name] {
                    let e = &entry.entry;
                    if e.tag && e.required == mandatory && !reaches(&edges, &e.id, name, 0)? {
                        let children = edges.entry(name).or_default();
                        if !children.contains(&e.id.as_str()) {
                            children.push(&e.id);
                        }
                    }
                }
            }
        }
        // Guava HashMultimap value sets start with capacity four (two expected
        // values). Their native bucket order matters when cycles are pruned.
        let edges = edges
            .into_iter()
            .map(|(key, children)| Ok((key, java_bucket_order(children, 4)?)))
            .collect::<Result<BTreeMap<_, _>, TagBuildError>>()?;
        let mut visited = BTreeSet::new();
        let mut sorted = Vec::new();
        for name in order {
            visit(name, &edges, &mut visited, &mut sorted, 0)?;
        }
        let mut result: BuiltTags<T> = BuiltTags {
            tags: BTreeMap::new(),
            missing: BTreeMap::new(),
        };
        for name in sorted {
            let Some(entries) = self.entries.get(name) else {
                continue;
            };
            let mut seen = BTreeSet::new();
            let mut values = Vec::new();
            let mut missing = Vec::new();
            for sourced in entries {
                let entry = &sourced.entry;
                if entry.tag {
                    if let Some(children) = result.tags.get(&entry.id) {
                        for value in children {
                            if seen.insert(value.clone()) {
                                values.push(value.clone());
                            }
                        }
                    } else if entry.required {
                        missing.push(sourced.clone());
                    }
                } else if let Some(value) = lookup(&entry.id, entry.required) {
                    if seen.insert(value.clone()) {
                        values.push(value);
                    }
                } else if entry.required {
                    missing.push(sourced.clone());
                }
            }
            if missing.is_empty() {
                result.tags.insert(name.into(), values);
            } else {
                result.missing.insert(name.into(), missing);
            }
        }
        Ok(result)
    }
}
fn reaches(
    edges: &BTreeMap<&str, Vec<&str>>,
    from: &str,
    target: &str,
    depth: usize,
) -> Result<bool, TagBuildError> {
    if depth > 256 {
        return Err(TagBuildError::DependencyDepth);
    }
    if let Some(children) = edges.get(from) {
        if children.contains(&target) {
            return Ok(true);
        }
        for child in children {
            if reaches(edges, child, target, depth + 1)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn visit<'a>(
    name: &'a str,
    edges: &BTreeMap<&'a str, Vec<&'a str>>,
    visited: &mut BTreeSet<&'a str>,
    sorted: &mut Vec<&'a str>,
    depth: usize,
) -> Result<(), TagBuildError> {
    if !visited.insert(name) {
        return Ok(());
    }
    if depth > 256 {
        return Err(TagBuildError::DependencyDepth);
    }
    if let Some(children) = edges.get(name) {
        for child in children {
            visit(child, edges, visited, sorted, depth + 1)?;
        }
    }
    sorted.push(name);
    Ok(())
}
fn hash(name: &str) -> u32 {
    let (namespace, path) = name.split_once(':').expect("canonical tag identifier");
    let raw = java_string_hash(namespace.encode_utf16())
        .wrapping_mul(31)
        .wrapping_add(java_string_hash(path.encode_utf16())) as u32;
    raw ^ (raw >> 16)
}
fn java_bucket_order<'a>(
    names: impl IntoIterator<Item = &'a str>,
    initial: usize,
) -> Result<Vec<&'a str>, TagBuildError> {
    let names: Vec<_> = names.into_iter().collect();
    let mut capacity = initial;
    for count in 1..=names.len() {
        let bucket = hash(names[count - 1]) as usize & (capacity - 1);
        let collisions = names[..count]
            .iter()
            .filter(|name| hash(name) as usize & (capacity - 1) == bucket)
            .count();
        if collisions >= 9 {
            if capacity < 64 {
                capacity *= 2;
            } else {
                return Err(TagBuildError::HashTreeBucket);
            }
        }
        if count > capacity * 3 / 4 {
            capacity *= 2;
        }
    }
    let mut ordered = names;
    ordered.sort_by_key(|name| hash(name) as usize & (capacity - 1));
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_replacement_keeps_entries_and_codec_error_can_create_empty_tag() {
        let mut definitions = TagDefinitions::default();
        definitions
            .apply_json("a", "first", r##"{"values":["one","two","one"]}"##)
            .unwrap();
        assert!(
            definitions
                .apply_json("a", "bad", r##"{"replace":true,"values":[false]}"##)
                .is_err()
        );
        assert!(definitions.apply_json("empty", "bad", "{}").is_err());
        assert!(definitions.apply_json("syntax", "bad", "{").is_err());
        let built = definitions.build(|id, _| Some(id.to_owned())).unwrap();
        assert_eq!(
            built.tags["minecraft:a"],
            vec!["minecraft:one", "minecraft:two"]
        );
        assert_eq!(built.tags["minecraft:empty"], Vec::<String>::new());
        assert!(!built.tags.contains_key("minecraft:syntax"));
        definitions
            .apply_json(
                "a",
                "replacement",
                r##"{"replace":true,"values":["three"]}"##,
            )
            .unwrap();
        let built = definitions.build(|id, _| Some(id.to_owned())).unwrap();
        assert_eq!(built.tags["minecraft:a"], vec!["minecraft:three"]);
    }
    #[test]
    fn required_edges_precede_optional_edges_and_missing_tags_do_not_poison_siblings() {
        let mut definitions = TagDefinitions::default();
        for (name, json) in [
            ("a", r##"{"values":[{"id":"#b","required":false},"one"]}"##),
            ("b", r##"{"values":["#a","two"]}"##),
            ("bad", r##"{"values":["absent","also_absent"]}"##),
            (
                "c",
                r##"{"values":[{"id":"#bad","required":false},"two"]}"##,
            ),
        ] {
            definitions.apply_json(name, "fixture", json).unwrap();
        }
        let built = definitions
            .build(|id, _| (!id.ends_with("absent")).then(|| id.to_owned()))
            .unwrap();
        assert_eq!(built.tags["minecraft:a"], vec!["minecraft:one"]);
        assert_eq!(
            built.tags["minecraft:b"],
            vec!["minecraft:one", "minecraft:two"]
        );
        assert_eq!(built.tags["minecraft:c"], vec!["minecraft:two"]);
        assert_eq!(
            built.missing["minecraft:bad"]
                .iter()
                .map(|e| e.entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["minecraft:absent", "minecraft:also_absent"]
        );
    }
}
