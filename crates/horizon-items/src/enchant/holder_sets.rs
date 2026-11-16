//! Load-owned holder-set binding shared by gameplay and registry synchronization.
use serde_json::Value;
use std::{collections::HashMap, path::Path};

/// Complete ordered registry and tag snapshots for enchantment holder-set fields.
pub struct HolderSetBinding {
    items: Vec<String>,
    enchantments: Vec<String>,
    item_tags: HashMap<String, Vec<String>>,
    enchantment_tags: HashMap<String, Vec<String>>,
}
impl HolderSetBinding {
    /// Capture the same item universe used by `Items::load`, including mod defaults.
    pub fn load(
        root: &Path,
        overlay: &horizon_world::loot::PackOverlay,
        defaults: &horizon_world::item_defaults::ItemDefaults,
        mut enchantments: Vec<String>,
    ) -> crate::Result<Self> {
        enchantments.sort();
        enchantments.dedup();
        Ok(Self {
            items: defaults.entries().keys().cloned().collect(),
            enchantments,
            item_tags: crate::items::load_tag_lists(root, "item", overlay)?,
            enchantment_tags: crate::items::load_tag_lists(root, "enchantment", overlay)?,
        })
    }
    /// Whether a definition needs binding; plain vanilla forms remain untouched.
    pub fn needed(value: &Value) -> bool {
        ["supported_items", "primary_items", "exclusive_set"]
            .iter()
            .any(|field| value[*field].is_object())
    }
    /// Validate objects once and replace them with ordered resolved IDs before publication.
    pub fn bind(&self, id: &str, value: &mut Value) -> crate::Result<()> {
        for field in ["supported_items", "primary_items", "exclusive_set"] {
            if !value[field].is_object() {
                continue;
            }
            let (registry, ids, tags) = if field == "exclusive_set" {
                ("enchantment", &self.enchantments, &self.enchantment_tags)
            } else {
                ("item", &self.items, &self.item_tags)
            };
            let universe = || ids.clone();
            let tag = |tag: &str| {
                Ok(tags
                    .get(&crate::recipe_sources::qualified(tag))
                    .cloned()
                    .unwrap_or_default())
            };
            let entry = |id: &str| Ok(vec![crate::recipe_sources::qualified(id)]);
            let lookup = horizon_world::holder_set::StringLookup {
                what: registry,
                ids: Some(&universe),
                tag: &tag,
                entry: &entry,
            };
            let resolved = horizon_world::holder_set::resolve(&value[field], &lookup)
                .map_err(|error| format!("enchantment {id} {field}: {error}"))?;
            value[field] = Value::Array(resolved.into_iter().map(Value::String).collect());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding() -> HolderSetBinding {
        HolderSetBinding {
            items: vec!["minecraft:stick".into(), "test:tool".into()],
            enchantments: vec!["test:a".into(), "test:b".into(), "test:c".into()],
            item_tags: HashMap::from([(
                "test:ordered".into(),
                vec!["test:tool".into(), "minecraft:stick".into()],
            )]),
            enchantment_tags: HashMap::new(),
        }
    }
    #[test]
    fn complete_universes_and_ordered_tags_are_bound_before_membership() {
        let mut definition = json!({
            "supported_items": {"type": "neoforge:any"},
            "primary_items": {"type": "neoforge:or", "values": ["#test:ordered", "test:tool"]},
            "exclusive_set": {"type": "neoforge:not", "value": "test:a"}
        });
        binding().bind("test:a", &mut definition).unwrap();
        assert_eq!(
            definition["supported_items"],
            json!(["minecraft:stick", "test:tool"])
        );
        assert_eq!(
            definition["primary_items"],
            json!(["test:tool", "minecraft:stick"])
        );
        assert_eq!(definition["exclusive_set"], json!(["test:b", "test:c"]));
        let entry =
            super::super::parse_entry("test:a", &definition, &HashMap::new(), &HashMap::new())
                .unwrap();
        let items = crate::items::Items::for_tests(HashMap::new());
        assert!(super::super::Enchantments::item_in(
            &items,
            &entry.supported,
            "test:tool"
        ));
        assert!(!super::super::Enchantments::item_in(
            &items,
            &entry.supported,
            "test:missing"
        ));
        let mut all = json!({"exclusive_set": {"type": "neoforge:any"}});
        binding().bind("test:a", &mut all).unwrap();
        assert_eq!(all["exclusive_set"], json!(["test:a", "test:b", "test:c"]));
    }
    #[test]
    fn malformed_objects_refuse_with_definition_and_field_before_publication() {
        let mut definition = json!({"supported_items": {"type": "neoforge:or", "values": 42}});
        let error = binding()
            .bind("test:broken", &mut definition)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("test:broken")
                && error.contains("supported_items")
                && error.contains("no values"),
            "{error}"
        );
    }
}
