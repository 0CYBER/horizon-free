use horizon_world::{
    item_defaults, loot,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    native_load_hooks::{self, NativeLoadHooks},
};
use serde_json::{Map, Value, json};
struct EmptyTags;
impl loot::LootTags for EmptyTags {
    fn members(&self, _: &str, _: &str) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn load_hooks_are_noops_before_install_and_use_first_callbacks_after() {
    let root = std::env::temp_dir().join(format!("horizon-load-hooks-{}", std::process::id()));
    let context = LoadContext::new(
        DataLoadOrigin::ClientPreload,
        std::sync::Arc::new(LoadInputs::new(Default::default(), root.join("settings"))),
        [],
    );
    let server_context = context.for_load(DataLoadOrigin::ServerDataLoad, []);
    let mut table = json!([]);
    let mut components = Map::new();
    loot::native_loot_table_load(&server_context, "table", &mut table);
    item_defaults::native_modify_default_components(&context, "item", &mut components);
    assert_eq!(table, json!([]));
    assert!(components.is_empty());

    let hooks = || NativeLoadHooks {
        item_ability_default: |_, _, _| false,
        loot_ability: |_, _, _| {},
        loot_table_load: |context, id, table| {
            assert_eq!(context.origin(), DataLoadOrigin::ServerDataLoad);
            if id == "minecraft:malformed" {
                *table = json!({"pools": [{"entries": [{
                    "type": "minecraft:tag",
                    "items": {"type": "neoforge:or", "values": 42}
                }]}]});
            } else {
                table.as_array_mut().unwrap().push(id.into());
            }
        },
        modify_default_components: |context, item, components| {
            assert_eq!(context.origin(), DataLoadOrigin::ClientPreload);
            components.insert("item".into(), Value::String(item.into()));
        },
        loot_function: |_, _, _, stack, _| stack.count += 1,
        loot_condition: |kind, _, _, holds, _| *holds = kind == "mymod:held",
    };
    assert!(native_load_hooks::install(hooks()).is_ok());
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            item_ability_default: |_, _, _| false,
            loot_ability: |_, _, _| {},
            loot_table_load: |_, _, table| *table = json!("second"),
            modify_default_components: |_, _, components| components.clear(),
            loot_function: |_, _, _, stack, _| stack.count = 0,
            loot_condition: |_, _, _, holds, _| *holds = false,
        })
        .is_err()
    );
    loot::native_loot_table_load(&server_context, "table", &mut table);
    item_defaults::native_modify_default_components(&context, "item", &mut components);
    assert_eq!(table, json!(["table"]));
    assert_eq!(components["item"], json!("item"));

    // Client/shared loads keep raw tables; each server load gets exactly one
    // callback per table, even when the raw map is already cached.
    let dir = root.join("data/minecraft/loot_table");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("items.json"), r#"{"probe:token":{}}"#).unwrap();
    std::fs::write(dir.join("test.json"), "[]").unwrap();
    let item_data = item_defaults::ItemData::load(&root, context).unwrap();
    // The server loot load owns its context even when the item data came from a client preload.
    assert_eq!(
        item_data.defaults().entries()["probe:token"]["item"],
        "probe:token"
    );
    let raw = loot::shared_tables(&root, &loot::PackOverlay::NONE).unwrap();
    assert_eq!(raw["minecraft:test"], json!([]));
    for _ in 0..2 {
        let mut server =
            loot::LootData::load(&root, &loot::PackOverlay::NONE, item_data.defaults()).unwrap();
        server.apply_native_table_loads(&server_context).unwrap();
        assert_eq!(server.tables()["minecraft:test"], json!(["minecraft:test"]));
        assert_eq!(raw["minecraft:test"], json!([]));
    }
    // A malformed native replacement refuses the whole candidate, not just
    // the bad field, and leaves the previously usable table snapshot intact.
    std::fs::write(dir.join("malformed.json"), r#"["original"]"#).unwrap();
    let mut server =
        loot::LootData::load(&root, &loot::PackOverlay::NONE, item_data.defaults()).unwrap();
    let previous = server.tables().clone();
    let error = server
        .apply_native_table_loads(&server_context)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("native loot_table minecraft:malformed")
            && error.contains("pools[0].entries[0].items")
            && error.contains("no values"),
        "{error}"
    );
    assert!(std::sync::Arc::ptr_eq(server.tables(), &previous));
    assert_eq!(server.tables()["minecraft:malformed"], json!(["original"]));
    assert_eq!(server.tables()["minecraft:test"], json!([]));
    assert_eq!(raw["minecraft:test"], json!([]));
    std::fs::remove_dir_all(&root).unwrap();

    let ctx = loot::LootContext {
        queried_table_id: None,
        queried_stack: None,
        entity: None,
        damage: None,
        player: None,
        tool: None,
        block: None,
        luck: 0.0,
        biome: None,
        seed: 0,
        place: None,
    };
    for (kind, expected) in [("mymod:held", true), ("mymod:other", false)] {
        let mut holds = !expected;
        loot::native_invoke_loot_condition(kind, &json!({}), &ctx, &mut holds, &EmptyTags);
        assert_eq!(holds, expected, "{kind}");
    }
}
