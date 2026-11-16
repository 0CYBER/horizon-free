//! A synced registry entry carries its holder sets in vanilla's forms: a pack
//! enchantment's NeoForge set objects (`neoforge:or`...) are resolved to the
//! id list a vanilla client decodes.
use serde_json::Value;
type Entries = Vec<((String, String), Value, bool)>;

/// Bind objects against the same complete defaults and ordered tags as gameplay.
pub(super) fn expand_enchantments(
    entries: &mut Entries,
    resources: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay,
    defaults: &horizon_world::item_defaults::ItemDefaults,
) -> Result<(), String> {
    use crate::enchant::HolderSetBinding;
    if !entries
        .iter()
        .any(|(_, value, _)| HolderSetBinding::needed(value))
    {
        return Ok(());
    }
    let enchantments = entries
        .iter()
        .map(|((ns, path), _, _)| format!("{ns}:{path}"))
        .collect();
    let binding = HolderSetBinding::load(resources, overlay, defaults, enchantments)
        .map_err(|e| e.to_string())?;
    for ((ns, path), value, _) in entries {
        binding
            .bind(&format!("{ns}:{path}"), value)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_set_object_reaches_clients_as_ids_and_plain_forms_are_untouched() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            json!({"minecraft:stick": {}, "minecraft:iron_sword": {}, "test:custom": {}})
                .to_string(),
        )
        .unwrap();
        let files = vec![(
            "tags/minecraft:item/swords".to_owned(),
            vec![
                json!({"values": ["test:custom", "minecraft:iron_sword"]})
                    .to_string()
                    .into_bytes(),
            ],
        )];
        let overlay = horizon_world::loot::PackOverlay {
            conditions: &horizon_world::conditions::LoadConditions::vanilla(),
            files: &files,
        };
        let mut entries = vec![
            (
                ("test".to_owned(), "sticky".to_owned()),
                json!({"supported_items": {"type": "neoforge:or",
                    "values": ["#minecraft:swords", "minecraft:stick"]},
                    "primary_items": {"type": "neoforge:any"},
                    "exclusive_set": {"type": "neoforge:any"}}),
                true,
            ),
            (
                ("test".to_owned(), "plain".to_owned()),
                json!({"supported_items": "#minecraft:swords"}),
                true,
            ),
        ];
        let defaults = horizon_world::item_defaults::read(root.path()).unwrap();
        expand_enchantments(&mut entries, root.path(), &overlay, &defaults).unwrap();
        assert_eq!(
            entries[0].1["supported_items"],
            json!(["test:custom", "minecraft:iron_sword", "minecraft:stick"])
        );
        assert_eq!(
            entries[0].1["primary_items"],
            json!(["minecraft:iron_sword", "minecraft:stick", "test:custom"])
        );
        assert_eq!(
            entries[0].1["exclusive_set"],
            json!(["test:plain", "test:sticky"])
        );
        assert_eq!(entries[1].1["supported_items"], json!("#minecraft:swords"));
    }
}
