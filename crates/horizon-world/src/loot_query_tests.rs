use super::*;
use serde_json::json;
use std::rc::Rc;
fn data(tables: Vec<(&str, Value)>) -> LootData {
    LootData {
        holder_sets: Default::default(),
        tables: Arc::new(tables.into_iter().map(|(k, v)| (k.into(), v)).collect()),
        modifiers: HashMap::new(),
        predicates: HashMap::new(),
        items: HashMap::new(),
        enchantments: vec![],
        tags: HashMap::new(),
        smelting: vec![],
    }
}
fn table(id: &str) -> Value {
    json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:loot_table_id","loot_table_id":id},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]})
}
#[test]
fn queried_id_is_written_after_root_and_first_wins() {
    for (guard, preset, count) in [
        ("minecraft:blocks/short_grass", None, 0),
        ("neoforge:unknown_loot_table", None, 1),
        (
            "minecraft:blocks/short_grass",
            Some("minecraft:blocks/short_grass"),
            1,
        ),
    ] {
        let data = data(vec![("minecraft:blocks/short_grass", table(guard))]);
        let query = Rc::new(query::LootQuery::default());
        if let Some(id) = preset {
            query.set_id(id);
        }
        let drops = query::with_query(query.clone(), || {
            data.roll(
                "minecraft:blocks/short_grass",
                &mut LegacyRandom::new(7),
                0.0,
                None,
                None,
            )
        });
        assert_eq!(drops.len(), count);
        if count == 1 {
            assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
        }
        assert_eq!(query.table_id(), "minecraft:blocks/short_grass");
    }
}
#[test]
fn queried_id_nested_and_unnamed_roots() {
    let data = data(vec![
        (
            "minecraft:chests/simple_dungeon",
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
        ),
        ("test:b", table("neoforge:unknown_loot_table")),
    ]);
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.roll(
            "minecraft:chests/simple_dungeon",
            &mut LegacyRandom::new(9),
            0.0,
            None,
            None,
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "minecraft:chests/simple_dungeon");
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.roll_inline(
            &table("neoforge:unknown_loot_table"),
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
    let query = Rc::new(query::LootQuery::default());
    let inline = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":table("neoforge:unknown_loot_table")}]}]});
    let drops = query::with_query(query.clone(), || {
        data.roll_inline(&inline, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
}

#[test]
fn queried_context_and_ability_reach_native_adapter() {
    crate::native_load_hooks::testing::install();
    let stack = || {
        let mut s = LootStack::new("minecraft:stick");
        s.count = 3;
        s
    };
    for (tool, expected) in [
        (None, 0),
        (Some("minecraft:stick"), 0),
        (Some("minecraft:shears"), 1),
        (Some("minecraft:stone_pickaxe"), 0),
        (Some("test:grant"), 1),
        (Some("test:deny"), 0),
    ] {
        for ability in ["shears_dig", "minecraft:shears_dig", "unknown_ability"] {
            let table = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":ability},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
            let data = data(vec![
                (
                    "test:a",
                    json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
                ),
                ("test:b", table),
            ]);
            let query = query::LootQuery::new(
                None,
                tool.map(|item| {
                    let mut s = stack();
                    s.item = item.into();
                    s
                }),
            );
            let query = Rc::new(query);
            let drops = query::with_query(query, || {
                data.roll("test:a", &mut LegacyRandom::new(9), 0.0, None, None)
            });
            assert_eq!(
                crate::native_load_hooks::testing::take_ability(),
                tool.map(|item| (item.to_owned(), ability.to_owned()))
            );
            assert_eq!(
                drops.len(),
                if ability == "shears_dig" { expected } else { 0 }
            );
        }
    }
    let table = json!({"pools":[{"rolls":1,"condition":{"type":"test:query"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let data = data(vec![
        (
            "test:a",
            json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":"test:b"}]}]}),
        ),
        ("test:b", table),
    ]);
    let query = query::LootQuery::new(Some(stack()), None);
    let query = Rc::new(query);
    query.set_id("minecraft:blocks/short_grass");
    let drops = query::with_query(query, || {
        data.roll("test:a", &mut LegacyRandom::new(9), 0.0, None, None)
    });
    assert_eq!(drops.len(), 1);
    let (id, stack) = crate::native_load_hooks::testing::take_context().expect("query callback");
    assert_eq!(id, "minecraft:blocks/short_grass");
    let stack = stack.expect("queried stack");
    assert_eq!((&*stack.item, stack.count), ("minecraft:stick", 3));
}

#[test]
#[ignore = "daily"]
fn queried_ability_tags_use_prepared_world_sets() {
    crate::native_load_hooks::testing::install();
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .expect("prepared resources"),
    );
    let defaults = crate::item_defaults::read(&root).unwrap();
    let table = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":"sword_sweep"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let data = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
    let query = Rc::new(query::LootQuery::new(
        None,
        Some(LootStack::new("minecraft:diamond_sword")),
    ));
    let drops = query::with_query(query, || {
        data.roll_inline(&table, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
    assert!(data.contains(
        "item",
        "minecraft:douses_campfires",
        "minecraft:diamond_shovel"
    ));
    assert!(!data.contains("item", "minecraft:swords", "minecraft:diamond_axe"));
    let files = [(
        "tags/minecraft:item/swords".to_owned(),
        vec![serde_json::to_vec(&json!({"replace":true,"values":["minecraft:stick"]})).unwrap()],
    )];
    let reloaded = LootData::load(&root, &PackOverlay::new(&files), &defaults).unwrap();
    assert!(!reloaded.contains("item", "minecraft:swords", "minecraft:diamond_sword"));
    assert!(reloaded.contains("item", "minecraft:swords", "minecraft:stick"));
    assert!(data.contains("item", "minecraft:swords", "minecraft:diamond_sword"));
}
