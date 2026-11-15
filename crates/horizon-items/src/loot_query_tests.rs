thread_local! {
    static OBSERVED_CONTEXT: std::cell::RefCell<Option<(String, Option<horizon_world::loot::LootStack>)>> = const { std::cell::RefCell::new(None) };
    static OBSERVED_ABILITY: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}
use super::*;
use horizon_core::random::LegacyRandom;
use horizon_world::loot::query;
use serde_json::json;
use std::rc::Rc;
fn data(tables: Vec<(&str, Value)>) -> Items {
    let mut items = Items::for_tests(HashMap::new());
    items.loot = Arc::new(tables.into_iter().map(|(k, v)| (k.into(), v)).collect());
    items
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
            data.loot_table(
                "minecraft:blocks/short_grass",
                false,
                None,
                &mut LegacyRandom::new(7),
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
        data.loot_table(
            "minecraft:chests/simple_dungeon",
            false,
            None,
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "minecraft:chests/simple_dungeon");
    let query = Rc::new(query::LootQuery::default());
    let drops = query::with_query(query.clone(), || {
        data.inline_loot_table(
            &table("neoforge:unknown_loot_table"),
            &mut LegacyRandom::new(9),
        )
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
    let query = Rc::new(query::LootQuery::default());
    let inline = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:loot_table","value":table("neoforge:unknown_loot_table")}]}]});
    let drops = query::with_query(query.clone(), || {
        data.inline_loot_table(&inline, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    assert_eq!(query.table_id(), "neoforge:unknown_loot_table");
}

#[test]
fn queried_context_and_ability_reach_native_adapter() {
    use horizon_world::native_load_hooks::{self, NativeLoadHooks};
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            item_ability_default: |item, ability, tags| match ability {
                "shears_dig" => item == "minecraft:shears",
                "sword_sweep" => tags.contains("item", "minecraft:swords", item),
                _ => false,
            },
            loot_table_load: |_, _, _| {},
            modify_default_components: |_, _, _| {},
            loot_function: |_, _, _, _, _| {},
            loot_condition: |kind, _, ctx, holds, _| {
                if kind == "test:query" {
                    OBSERVED_CONTEXT.with(|v| {
                        *v.borrow_mut() =
                            Some((ctx.table_id().to_owned(), ctx.queried_stack.clone()))
                    });
                    *holds = true;
                }
            },
            loot_ability: |tool, ability, holds| {
                OBSERVED_ABILITY
                    .with(|v| *v.borrow_mut() = Some((tool.item.clone(), ability.to_owned())));
                if tool.item == "test:grant" && ability == "shears_dig" {
                    *holds = true;
                }
                if tool.item == "test:deny" {
                    *holds = false;
                }
            },
        })
        .is_ok()
    );
    let stack = || {
        super::loot_of(&ItemStack {
            item: Arc::from("minecraft:stick"),
            count: 3,
            damage: 0,
            extra: None,
        })
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
            let held = tool.map(|item| ItemStack {
                item: Arc::from(item),
                count: 1,
                damage: 0,
                extra: None,
            });
            let drops = query::with_query(query, || {
                data.loot_table("test:a", false, held.as_ref(), &mut LegacyRandom::new(9))
            });
            assert_eq!(
                OBSERVED_ABILITY.with(|v| v.take()),
                tool.map(|item| (item.to_owned(), ability.to_owned()))
            );
            assert_eq!(
                drops.len(),
                if ability == "shears_dig" { expected } else { 0 }
            );
        }
    }
    let empty_shears = ItemStack {
        item: Arc::from("minecraft:shears"),
        count: 0,
        damage: 0,
        extra: None,
    };
    let guard = json!({"pools":[{"rolls":1,"condition":{"type":"neoforge:can_item_perform_ability","ability":"shears_dig"},"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]});
    let empty = data(vec![("test:a", guard)]).loot_table(
        "test:a",
        false,
        Some(&empty_shears),
        &mut LegacyRandom::new(9),
    );
    assert_eq!(empty.len(), 0);
    assert_eq!(OBSERVED_ABILITY.with(|v| v.take()), None);
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
        data.loot_table("test:a", false, None, &mut LegacyRandom::new(9))
    });
    assert_eq!(drops.len(), 1);
    let (id, stack) = OBSERVED_CONTEXT.with(|v| v.take()).expect("query callback");
    assert_eq!(id, "minecraft:blocks/short_grass");
    let stack = stack.expect("queried stack");
    assert_eq!((&*stack.item, stack.count), ("minecraft:stick", 3));
}

#[test]
fn queried_ability_default_sets_have_literal_outcomes() {
    let items = Items::for_tests(HashMap::new());
    for (item, abilities) in [
        (
            "minecraft:shears",
            &[
                "shears_dig",
                "shears_harvest",
                "shears_remove_armor",
                "shears_carve",
                "shears_disarm",
                "shears_trim",
            ][..],
        ),
        ("minecraft:fishing_rod", &["fishing_rod_cast"][..]),
        ("minecraft:trident", &["trident_throw"][..]),
        ("minecraft:brush", &["brush_brush"][..]),
        ("minecraft:flint_and_steel", &["firestarter_light"][..]),
        ("minecraft:fire_charge", &["firestarter_light"][..]),
        ("minecraft:spyglass", &["spyglass_scope"][..]),
    ] {
        for ability in abilities {
            assert!(
                items.vanilla_can_perform(item, ability),
                "{item}: {ability}"
            );
        }
    }
    for item in [
        "minecraft:shears",
        "minecraft:stone_pickaxe",
        "minecraft:shield",
        "minecraft:bow",
        "minecraft:crossbow",
        "minecraft:mace",
        "minecraft:diamond_axe",
        "minecraft:diamond_hoe",
        "minecraft:diamond_shovel",
    ] {
        for ability in [
            "unknown_ability",
            "minecraft:shears_dig",
            "SHEARS_DIG",
            "shield_block",
            "axe_strip",
            "pickaxe_dig",
        ] {
            assert!(!items.vanilla_can_perform(item, ability));
        }
    }
    assert!(!items.vanilla_can_perform("minecraft:stone_pickaxe", "shears_dig"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_sword", "sword_sweep"));
}
#[test]
#[ignore = "daily"]
fn queried_ability_tags_use_prepared_item_sets() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .or_else(|| std::env::var_os("HORIZON_RESOURCES"))
            .expect("prepared resources"),
    );
    let (registry, _) = BlockRegistry::from_facts_json_with_flags(
        std::fs::File::open(root.join("facts.json")).unwrap(),
    )
    .unwrap();
    let facts = crate::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::load(&root, &registry, &facts, &PackOverlay::NONE, &defaults).unwrap();
    assert!(items.vanilla_can_perform("minecraft:diamond_sword", "sword_sweep"));
    assert!(items.vanilla_can_perform("minecraft:diamond_shovel", "shovel_douse"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_axe", "sword_sweep"));
    assert!(!items.vanilla_can_perform("minecraft:diamond_sword", "minecraft:sword_sweep"));
    let guard = json!({"type":"neoforge:can_item_perform_ability","ability":"sword_sweep"});
    let props = horizon_world::chunk::BlockProperties::new();
    let tool = ItemStack {
        item: Arc::from("minecraft:diamond_sword"),
        count: 1,
        damage: 0,
        extra: None,
    };
    let context = LootContext {
        query: query::current_query(),
        items: &items,
        block: "",
        properties: &props,
        held: Some(&tool),
        killed_by_player: false,
        explosion_radius: None,
        this_entity: false,
        entity_levels: None,
        block_levels: None,
        target: None,
        location: None,
        keep_empty: false,
    };
    assert!(context.condition(&guard, &mut LegacyRandom::new(9)));
    for invalid in [
        json!({"type":"neoforge:can_item_perform_ability"}),
        json!({"type":"neoforge:can_item_perform_ability","ability":7}),
        json!({"type":"neoforge:can_item_perform_ability","ability":"unknown_ability"}),
    ] {
        assert!(!context.condition(&invalid, &mut LegacyRandom::new(9)));
    }
}
