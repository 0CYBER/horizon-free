//! The items' default names where they aren't their description id's translation.
use horizon_client_core::text::Text;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// Each item whose default `minecraft:item_name` (the loaded `items.json`) is not the
/// translation of its description id, `item.<ns>.<path>` or `block.<ns>.<path>`, as every
/// vanilla item's is: what a content mod declares, such as a translation with arguments.
/// Each item-data load replaces it.
static NAMES: RwLock<Option<Arc<HashMap<String, Value>>>> = RwLock::new(None);

/// Keep the default names among `defaults` (item id, its default components) that are not
/// the item's description id.
pub fn set<'a>(defaults: impl IntoIterator<Item = (&'a String, &'a Value)>) {
    let names = defaults
        .into_iter()
        .filter_map(|(id, components)| {
            let name = components.get("minecraft:item_name")?;
            let (namespace, path) = id.split_once(':')?;
            let own = |kind: &str| {
                let key = format!("{kind}.{namespace}.{}", path.replace('/', "."));
                serde_json::json!({ "translate": key })
            };
            (*name != own("item") && *name != own("block")).then(|| (id.clone(), name.clone()))
        })
        .collect();
    *NAMES.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(names));
}

/// `item`'s default name, when it is not its description id's translation.
pub fn of(item: &str) -> Option<Text> {
    let names = NAMES.read().unwrap_or_else(|e| e.into_inner()).clone()?;
    names
        .get(item)
        .map(horizon_client_core::scoreboard::comp_text)
}

#[cfg(test)]
mod tests {
    use crate::items::ItemStack;

    /// A content item's declared name names its stacks; an item named by its description
    /// id, and a stack's own name, keep theirs.
    #[test]
    fn a_declared_default_name_names_the_stack() {
        let defaults: Vec<(String, serde_json::Value)> = vec![
            (
                "stick_test:table".into(),
                serde_json::json!({"minecraft:item_name": {
                    "translate": "item.stick_test.table",
                    "fallback": "%s",
                    "with": [{"text": "Crafting Table"}],
                }}),
            ),
            (
                "stick_test:plain".into(),
                serde_json::json!({"minecraft:item_name": {"translate": "item.stick_test.plain"}}),
            ),
        ];
        super::set(defaults.iter().map(|(id, c)| (id, c)));
        assert_eq!(
            ItemStack::new("stick_test:table", 1).item_name().as_deref(),
            Some("Crafting Table")
        );
        assert_eq!(ItemStack::new("stick_test:plain", 1).item_name(), None);
        let renamed = crate::item_data::parse_stack(
            r#"{id:"stick_test:table",count:1,components:{"minecraft:item_name":"Workbench"}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(renamed.item_name().as_deref(), Some("Workbench"));
    }
}
