//! Code item identities available while reading a package's content files.
use horizon_mod_package::{Manifest, content::Item};

pub(super) fn owned_item_ids(manifest: &Manifest, items: &[Item]) -> Vec<String> {
    items
        .iter()
        .filter(|item| {
            item.id.split_once(':').is_some_and(|(namespace, _)| {
                manifest.owned_namespaces().any(|owned| owned == namespace)
            })
        })
        .map(|item| item.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_mod_package::Content;

    #[test]
    fn file_remainders_can_name_code_items_in_any_owned_namespace() {
        let manifest = Manifest::parse(
            r#"
            id = "porter"
            name = "Porter"
            version = "1.0.0"
            api = "0.2"
            side = "both"
            capabilities = ["registries"]
            namespaces = ["legacy"]
        "#,
        )
        .unwrap();
        let items = ["porter:spoon", "legacy:wrench", "foreign:tool"].map(|id| Item {
            id: id.into(),
            template: "minecraft:stick".into(),
            ..Item::default()
        });
        let declared = owned_item_ids(&manifest, &items);
        assert_eq!(declared, ["porter:spoon", "legacy:wrench"]);
        let root = std::env::temp_dir().join(format!(
            "horizon-owned-code-items-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let folder = root.join("data/legacy/horizon/item");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("soup.json");
        std::fs::write(
            &file,
            br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"legacy:wrench"}"#,
        )
        .unwrap();
        let content = Content::read_declaring(&root, &manifest, &declared).unwrap();
        assert_eq!(content.items[0].id, "legacy:soup");
        assert_eq!(
            content.items[0].crafting_remainder.as_deref(),
            Some("legacy:wrench")
        );
        std::fs::write(
            &file,
            br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"foreign:tool"}"#,
        )
        .unwrap();
        let error = Content::read_declaring(&root, &manifest, &declared).unwrap_err();
        assert!(
            error.contains("foreign:tool") && error.contains("crafting_remainder"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
