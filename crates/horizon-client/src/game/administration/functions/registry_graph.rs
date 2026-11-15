//! References in the reloadable loot registries, checked before a candidate pack stack is admitted.
//!
//! This follows the holder positions in the 26.3 codecs, rather than treating arbitrary strings
//! in JSON as references. Binding precedes cycle validation (`RegistryDataLoader` and
//! `ReloadableServerRegistries`); tags obey pack order, `replace`, and optional entries.
mod fields;

use horizon_client::datapack::DataPack;
use horizon_client_core::identifier::parse_identifier;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Registry {
    Predicate,
    Modifier,
    Table,
    Float,
    Int,
    Slots,
    Advancement,
}

impl Registry {
    const ALL: [Self; 7] = [
        Self::Predicate,
        Self::Modifier,
        Self::Table,
        Self::Float,
        Self::Int,
        Self::Slots,
        Self::Advancement,
    ];

    fn directory(self) -> &'static str {
        match self {
            Self::Predicate => "predicate",
            Self::Modifier => "item_modifier",
            Self::Table => "loot_table",
            Self::Float => "context_float_provider",
            Self::Int => "context_int_provider",
            Self::Slots => "slot_source",
            Self::Advancement => "advancement",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    registry: Registry,
    id: String,
    tag: bool,
}

impl Key {
    fn element(registry: Registry, id: String) -> Self {
        Self {
            registry,
            id,
            tag: false,
        }
    }

    fn display_id(&self) -> String {
        format!("{}{}", if self.tag { "#" } else { "" }, self.id)
    }

    fn path(&self) -> String {
        format!(
            "{{{}@minecraft:{}}}",
            self.display_id(),
            self.registry.directory()
        )
    }
}

struct Reference {
    registry: Registry,
    name: String,
    path: String,
    tag: bool,
    // Bind every codec holder; traverse only those visited by the owning type's validate method.
    follow: bool,
}

struct Edge {
    target: Key,
    path: String,
    follow: bool,
}

#[derive(Debug)]
pub(in crate::game) struct Error {
    pub lines: Vec<String>,
    pub binding: bool,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.lines.join("; "))
    }
}

impl std::error::Error for Error {}

/// The resolved predicate tags also supply the existing predicate evaluator's holder lists.
#[derive(Default)]
pub(super) struct BoundTags(BTreeMap<Key, Vec<String>>);

impl BoundTags {
    pub(super) fn expand_predicates(&self, predicates: &mut BTreeMap<String, Value>) {
        for value in predicates.values_mut() {
            self.expand_condition(value);
        }
    }

    fn expand_condition(&self, value: &mut Value) {
        match value.get("type").and_then(Value::as_str).map(fields::kind) {
            Some("all_of" | "any_of") => {
                if let Some(name) = value["terms"].as_str().and_then(|s| s.strip_prefix('#'))
                    && let Some(id) = parse_identifier(name)
                    && let Some(entries) = self.0.get(&Key {
                        registry: Registry::Predicate,
                        id,
                        tag: true,
                    })
                {
                    value["terms"] =
                        Value::Array(entries.iter().cloned().map(Value::String).collect());
                }
                match value.get_mut("terms") {
                    Some(Value::Array(terms)) => {
                        for term in terms {
                            self.expand_condition(term);
                        }
                    }
                    Some(term) => self.expand_condition(term),
                    None => {}
                }
            }
            Some("inverted") => {
                if let Some(term) = value.get_mut("term") {
                    self.expand_condition(term);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Graph {
    values: BTreeMap<Key, Value>,
    tags: BTreeMap<Key, Vec<(String, bool)>>,
}

fn file_id(path: &str, directory: &str) -> Option<String> {
    let (namespace, path) = path.strip_prefix("data/")?.split_once('/')?;
    let path = path
        .strip_prefix(directory)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    parse_identifier(&format!("{namespace}:{path}"))
}

fn tag_entries(value: &Value) -> Option<Vec<(String, bool)>> {
    if value
        .get("replace")
        .is_some_and(|replace| !replace.is_boolean())
    {
        return None;
    }
    value
        .get("values")?
        .as_array()?
        .iter()
        .map(|entry| {
            let (name, required) = if let Some(name) = entry.as_str() {
                (name, true)
            } else {
                let name = entry.get("id")?.as_str()?;
                let required = match entry.get("required") {
                    Some(required) => required.as_bool()?,
                    None => true,
                };
                (name, required)
            };
            parse_identifier(name.strip_prefix('#').unwrap_or(name))?;
            Some((name.to_owned(), required))
        })
        .collect()
}

/// `RegistryDataLoader.load`: read the selected resources and bind their holders before validation.
/// Includes the same built-in registries as the candidate function library.
pub(super) fn check(packs: &[&DataPack]) -> Result<BoundTags, Error> {
    let mut graph = Graph::default();
    let mut errors = BTreeMap::new();
    for pack in packs {
        for registry in Registry::ALL {
            let directory = registry.directory();
            for path in pack.list_folder(directory) {
                let Some(id) = file_id(&path, directory) else {
                    continue;
                };
                let Some(bytes) = pack.read(&path) else {
                    continue;
                };
                let key = Key::element(registry, id);
                match serde_json::from_slice(&bytes) {
                    Ok(value) => {
                        errors.remove(&key);
                        graph.values.insert(key, value);
                    }
                    Err(error) => {
                        graph.values.remove(&key);
                        errors.insert(
                            key.clone(),
                            format!("Couldn't parse data file {}: {error}", key.path()),
                        );
                    }
                }
            }
            let directory = format!("tags/{directory}");
            for path in pack.list_folder(&directory) {
                let Some(id) = file_id(&path, &directory) else {
                    continue;
                };
                let Some(value) = pack
                    .read(&path)
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                else {
                    continue;
                };
                // TagFile's codec rejects the whole file if one entry is malformed.
                let Some(additions) = tag_entries(&value) else {
                    continue;
                };
                let entries = graph
                    .tags
                    .entry(Key {
                        registry,
                        id,
                        tag: true,
                    })
                    .or_default();
                if value["replace"].as_bool() == Some(true) {
                    entries.clear();
                }
                entries.extend(additions);
            }
        }
    }
    if !errors.is_empty() {
        return Err(Error {
            lines: errors.into_values().collect(),
            binding: true,
        });
    }
    graph.validate()
}

impl Graph {
    /// `TagLoader.build`: resolve nested tags; missing optional entries contribute no holders.
    fn resolve_tag(
        &self,
        key: &Key,
        active: &mut BTreeSet<Key>,
        resolved_tags: &mut BTreeMap<Key, Option<Vec<String>>>,
    ) -> Option<Vec<String>> {
        if let Some(resolved) = resolved_tags.get(key) {
            return resolved.clone();
        }
        let entries = self.tags.get(key)?;
        if !active.insert(key.clone()) {
            return None;
        }
        let mut resolved = Vec::new();
        for (name, required) in entries {
            let nested = name.starts_with('#');
            let id = parse_identifier(name.strip_prefix('#').unwrap_or(name));
            let target = id.map(|id| Key {
                registry: key.registry,
                id,
                tag: nested,
            });
            let values = target.and_then(|target| {
                if nested {
                    self.resolve_tag(&target, active, resolved_tags)
                } else {
                    // A required entry uses the writable registry lookup, creating an unbound
                    // holder when missing. Registry.freeze reports that value, not a missing tag.
                    (*required || self.values.contains_key(&target)).then_some(vec![target.id])
                }
            });
            match values {
                Some(values) => {
                    for id in values {
                        if !resolved.contains(&id) {
                            resolved.push(id);
                        }
                    }
                }
                None if *required => {
                    active.remove(key);
                    resolved_tags.insert(key.clone(), None);
                    return None;
                }
                None => {}
            }
        }
        active.remove(key);
        resolved_tags.insert(key.clone(), Some(resolved.clone()));
        Some(resolved)
    }

    /// `ReloadableServerRegistries.validateLootRegistries`: check the fully bound holder graph.
    fn validate(&self) -> Result<BoundTags, Error> {
        let mut resolved_tags = BTreeMap::new();
        for key in self.tags.keys() {
            let _ = self.resolve_tag(key, &mut BTreeSet::new(), &mut resolved_tags);
        }
        let tags = BoundTags(
            resolved_tags
                .into_iter()
                .filter_map(|(key, ids)| ids.map(|ids| (key, ids)))
                .collect(),
        );
        let mut graph = BTreeMap::<Key, Vec<Edge>>::new();
        let mut missing = BTreeMap::<(Registry, bool), BTreeSet<String>>::new();
        let mut errors = Vec::new();
        // Writable lookups create every required element holder, even in a tag whose other
        // entries fail to resolve. Freeze then reports these before unbound tag holders.
        for (key, entries) in &self.tags {
            for (name, required) in entries {
                if *required
                    && !name.starts_with('#')
                    && let Some(id) = parse_identifier(name)
                    && !self
                        .values
                        .contains_key(&Key::element(key.registry, id.clone()))
                {
                    missing.entry((key.registry, false)).or_default().insert(id);
                }
            }
        }
        for (key, value) in &self.values {
            let mut edges = Vec::new();
            let references = match fields::references(value, key.registry) {
                Ok(references) => references,
                Err(problems) => {
                    errors.extend(problems.into_iter().map(|problem| {
                        format!("Couldn't parse data file {}{problem}", key.path())
                    }));
                    continue;
                }
            };
            for reference in references {
                let Some(id) = parse_identifier(&reference.name) else {
                    errors.push(format!(
                        "Couldn't parse data file {}{}: Invalid identifier {}",
                        key.path(),
                        reference.path,
                        reference.name
                    ));
                    continue;
                };
                let target = Key {
                    registry: reference.registry,
                    id,
                    tag: reference.tag,
                };
                let bound = if target.tag {
                    tags.0.contains_key(&target)
                } else {
                    self.values.contains_key(&target)
                };
                if !bound {
                    missing
                        .entry((target.registry, target.tag))
                        .or_default()
                        .insert(target.id.clone());
                }
                edges.push(Edge {
                    target,
                    path: reference.path,
                    follow: reference.follow,
                });
            }
            graph.insert(key.clone(), edges);
        }
        for ((registry, tag), ids) in missing {
            errors.push(format!(
                "Unbound {} in registry ResourceKey[minecraft:root / minecraft:{}]: [{}]",
                if tag { "tags" } else { "values" },
                registry.directory(),
                ids.into_iter().collect::<Vec<_>>().join(", "),
            ));
        }
        if !errors.is_empty() {
            return Err(Error {
                lines: errors,
                binding: true,
            });
        }
        for (key, entries) in &tags.0 {
            graph.insert(
                key.clone(),
                entries
                    .iter()
                    .enumerate()
                    .map(|(index, id)| Edge {
                        target: Key::element(key.registry, id.clone()),
                        path: format!("[{index}]"),
                        follow: true,
                    })
                    .collect(),
            );
        }
        let mut acyclic = BTreeSet::new();
        for root in self
            .values
            .keys()
            .filter(|key| key.registry != Registry::Advancement)
        {
            walk(
                &graph,
                root,
                &root.path(),
                &mut BTreeSet::new(),
                &mut acyclic,
                &mut errors,
            );
        }
        if errors.is_empty() {
            Ok(tags)
        } else {
            Err(Error {
                lines: errors,
                binding: false,
            })
        }
    }
}

fn walk(
    graph: &BTreeMap<Key, Vec<Edge>>,
    key: &Key,
    path: &str,
    active: &mut BTreeSet<Key>,
    acyclic: &mut BTreeSet<Key>,
    errors: &mut Vec<String>,
) {
    if acyclic.contains(key) {
        return;
    }
    active.insert(key.clone());
    let before = errors.len();
    for edge in &graph[key] {
        if !edge.follow {
            continue;
        }
        let here = format!("{path}{}", edge.path);
        if active.contains(&edge.target) {
            errors.push(format!(
                "Found loot table element validation problem in {here}: {} of type minecraft:{} is recursively called",
                edge.target.display_id(), edge.target.registry.directory(),
            ));
        } else {
            walk(
                graph,
                &edge.target,
                &format!("{here}->{}", edge.target.path()),
                active,
                acyclic,
                errors,
            );
        }
    }
    active.remove(key);
    if before == errors.len() {
        acyclic.insert(key.clone());
    }
}

#[cfg(test)]
mod tests;
