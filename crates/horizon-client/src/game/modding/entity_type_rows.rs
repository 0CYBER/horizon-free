//! The entity types of a resource root's `entity-types.json`: vanilla's and the content mods' (each
//! mod type has its template's row). A living entity's row lists its default attributes, so a row
//! with `attributes` is a living type (`LivingEntity`), and one without is not (a boat, an arrow, a
//! display). Read once per root: the file doesn't change while the game runs.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

/// The entity types under one root, each list sorted.
#[derive(Debug, Default, PartialEq)]
pub(in crate::game) struct EntityTypeRows {
    /// Every entity type (`minecraft:boat`), namespaced as commands name it.
    pub(in crate::game) ids: Vec<String>,
    /// The living ones.
    living: Vec<String>,
}

impl EntityTypeRows {
    /// The rows of `entity-types.json`'s text; none when it isn't an object.
    pub(in crate::game) fn parse(bytes: &[u8]) -> Self {
        let rows = serde_json::from_slice::<serde_json::Value>(bytes).unwrap_or_default();
        let mut out = Self::default();
        for (id, row) in rows.as_object().into_iter().flatten() {
            let id = horizon_client::command::args::normalize(id);
            if row.get("attributes").is_some() {
                out.living.push(id.clone());
            }
            out.ids.push(id);
        }
        out.ids.sort();
        out.living.sort();
        out
    }

    /// Whether `id` is an entity type.
    pub(in crate::game) fn contains(&self, id: &str) -> bool {
        self.ids.binary_search_by(|t| t.as_str().cmp(id)).is_ok()
    }

    /// Whether `id` is a living entity type.
    pub(in crate::game) fn is_living(&self, id: &str) -> bool {
        self.living.binary_search_by(|t| t.as_str().cmp(id)).is_ok()
    }
}

/// Each root's rows, read on first use.
static ROWS: Mutex<Vec<(PathBuf, Arc<EntityTypeRows>)>> = Mutex::new(Vec::new());

/// The entity types under `root` (none when its `entity-types.json` is missing or unreadable).
pub(in crate::game) fn entity_type_rows(root: &Path) -> Arc<EntityTypeRows> {
    let mut cache = ROWS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, rows)) = cache.iter().find(|(cached, _)| cached == root) {
        return rows.clone();
    }
    let bytes = std::fs::read(root.join("entity-types.json")).unwrap_or_default();
    let rows = Arc::new(EntityTypeRows::parse(&bytes));
    cache.push((root.to_owned(), rows.clone()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_with_attributes_is_a_living_type() {
        let rows = EntityTypeRows::parse(
            br#"{"minecraft:oak_boat": {"width": 1.375},
                 "minecraft:zombie": {"width": 0.6, "attributes": {"minecraft:armor": 2.0}},
                 "ruby:golem": {"template": "minecraft:zombie", "attributes": {}}}"#,
        );
        assert_eq!(
            rows.ids,
            ["minecraft:oak_boat", "minecraft:zombie", "ruby:golem"]
        );
        assert!(rows.contains("minecraft:oak_boat"));
        assert!(!rows.contains("nope:ghost"));
        assert!(!rows.is_living("minecraft:oak_boat"));
        assert!(rows.is_living("minecraft:zombie"));
        assert!(rows.is_living("ruby:golem"));
    }
}
