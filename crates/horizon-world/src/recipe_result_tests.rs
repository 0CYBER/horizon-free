use super::*;

/// `ItemStackTemplate.CODEC` reads a stack's fields or a bare item id.
#[test]
fn a_result_is_an_object_with_an_id_or_a_bare_item_id() {
    let id = |json: &str| {
        let v: Value = serde_json::from_str(json).unwrap();
        recipe_result_id(&v)
    };
    assert_eq!(
        id(r#"{"id": "ruby:ruby", "count": 3}"#).as_deref(),
        Some("ruby:ruby")
    );
    assert_eq!(id(r#""ruby:ruby""#).as_deref(), Some("ruby:ruby"));
    // No `id` (a transmuting recipe keeps its input's item), the `item` key of older
    // versions and anything that is neither an object nor a string name no item.
    assert_eq!(id(r#"{"count": 2}"#), None);
    assert_eq!(id(r#"{"item": "minecraft:stick"}"#), None);
    assert_eq!(id(r#"{"id": 7}"#), None);
    assert_eq!(id("null"), None);
    assert_eq!(id("[]"), None);
}

/// `Item.CODEC` reads an id through `Identifier.parse`: a bare id names `minecraft:<id>`.
#[test]
fn an_item_id_written_without_a_namespace_names_minecraft() {
    let id = |json: &str| {
        let v: Value = serde_json::from_str(json).unwrap();
        recipe_result_id(&v)
    };
    for written in ["stick", ":stick", "minecraft:stick"] {
        let bare = format!("\"{written}\"");
        let in_object = format!(r#"{{"id": "{written}", "count": 3}}"#);
        assert_eq!(id(&bare).as_deref(), Some("minecraft:stick"), "{bare}");
        assert_eq!(
            id(&in_object).as_deref(),
            Some("minecraft:stick"),
            "{in_object}"
        );
    }
    // An id `Identifier.parse` refuses stays as written; the load decides whether it loads.
    assert_eq!(id(r#""Stick""#).as_deref(), Some("Stick"));
}

/// `SmeltItemFunction.run` looks up a smelting recipe whose ingredient
/// `AbstractCookingRecipe` read via `Ingredient.CODEC` (`HolderSetCodec`):
/// a bare id or `#tag` names `minecraft`'s, so the list `["iron_ore"]` smelts
/// `minecraft:iron_ore` and `#ores` resolves the `minecraft:ores` tag.
#[test]
fn bare_smelting_ingredient_names_minecrafts() {
    let root = std::env::temp_dir().join(format!("loot-smelt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let recipe_dir = root.join("data/minecraft/recipe");
    std::fs::create_dir_all(&recipe_dir).unwrap();
    std::fs::write(
        recipe_dir.join("bare.json"),
        r#"{"type":"minecraft:smelting","ingredient":["iron_ore"],"result":{"id":"minecraft:iron_ingot","count":1}}"#,
    )
    .unwrap();
    let tag_dir = root.join("data/minecraft/tags/item");
    std::fs::create_dir_all(&tag_dir).unwrap();
    std::fs::write(
        tag_dir.join("ores.json"),
        r#"{"values":["minecraft:copper_ore"]}"#,
    )
    .unwrap();
    std::fs::write(
        recipe_dir.join("tag.json"),
        r##"{"type":"minecraft:smelting","ingredient":"#ores","result":{"id":"minecraft:copper_ingot","count":1}}"##,
    )
    .unwrap();
    let loot_dir = root.join("data/minecraft/loot_table");
    std::fs::create_dir_all(&loot_dir).unwrap();
    for (name, item) in [
        ("iron_ore", "minecraft:iron_ore"),
        ("copper_ore", "minecraft:copper_ore"),
    ] {
        std::fs::write(
            loot_dir.join(format!("{name}.json")),
            format!(
                r#"{{"pools":[{{"rolls":1,"entries":[{{"type":"minecraft:item","name":"{item}","modifier":[{{"type":"minecraft:furnace_smelt"}}]}}]}}]}}"#
            ),
        )
        .unwrap();
    }
    std::fs::write(root.join("items.json"), "{}").unwrap();
    let defaults = crate::item_defaults::read(&root).unwrap();
    let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
    let drops = data.roll(
        "minecraft:iron_ore",
        &mut LegacyRandom::new(1),
        0.0,
        None,
        None,
    );
    assert_eq!(drops.len(), 1);
    assert_eq!(&*drops[0].item, "minecraft:iron_ingot");
    let drops = data.roll(
        "minecraft:copper_ore",
        &mut LegacyRandom::new(1),
        0.0,
        None,
        None,
    );
    assert_eq!(drops.len(), 1);
    assert_eq!(&*drops[0].item, "minecraft:copper_ingot");
    let _ = std::fs::remove_dir_all(&root);
}
