use super::*;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
fn context() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}
#[test]
fn difference_keeps_default_sword_and_tests_concrete_damage() {
    let items = Items::empty();
    let json = serde_json::json!({"type":"neoforge:difference","base":"minecraft:diamond_sword","subtracted":{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}});
    let ingredient = Ingredient::parse(&json, &items).unwrap();
    assert_eq!(ingredient.order, ["minecraft:diamond_sword"]);
    assert!(!ingredient.is_simple());
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(ingredient.test_in(&ctx, &sword));
    sword.damage = 5;
    assert!(!ingredient.test_in(&ctx, &sword));
    let root = tempfile::tempdir().unwrap();
    let recipes=crate::crafting::Recipes::from_sources(root.path(), &items, &horizon_world::loot::PackOverlay::NONE,
        vec![("fixture:difference".into(),serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[json],"result":{"id":"minecraft:diamond"}}))],&load).unwrap();
    assert_eq!(recipes.crafting.len(), 1);
}
#[test]
fn intersection_filters_default_stacks_without_losing_concrete_predicate() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:damage":0})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let parse = |damage| {
        Ingredient::parse(
            &serde_json::json!({"type":"neoforge:intersection","children":["minecraft:diamond_sword",{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":damage}}]}),
            &items,
        )
    };
    let only_damaged = parse(5).unwrap();
    assert!(only_damaged.order.is_empty());
    assert!(only_damaged.items.contains("minecraft:diamond_sword"));
    let ingredient = parse(0).unwrap();
    assert!(!ingredient.is_simple());
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(ingredient.test_in(&ctx, &sword));
    sword.damage = 5;
    assert!(!ingredient.test_in(&ctx, &sword));
    assert!(only_damaged.test_in(&ctx, &sword));
    sword.damage = 0;
    assert!(!only_damaged.test_in(&ctx, &sword));
}
#[test]
#[ignore = "daily"]
fn malformed_and_unknown_component_patches_reject_whole_recipe() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
    let load = context();
    for patch in [
        serde_json::json!({"!fixture:not_registered":{}}),
        serde_json::json!({"minecraft:damage":"not a number"}),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:diamond_sword","components":patch});
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes=crate::crafting::Recipes::from_sources(&root,&items,&horizon_world::loot::PackOverlay::NONE,vec![("fixture:invalid_component".into(),serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}}))],&load).unwrap();
        assert!(recipes.crafting.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}
#[test]
fn component_display_is_patched_and_candidate_matching_allocates_nothing() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([
        ("minecraft:diamond_sword".into(), crate::json_nbt::json_to_compound(serde_json::json!({"minecraft:damage":0,"minecraft:max_damage":1561,"minecraft:max_stack_size":1}).as_object().unwrap()).unwrap())
    ]));
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5,"minecraft:max_damage":1561,"minecraft:max_stack_size":1},"strict":true}),&items).unwrap();
    assert_eq!(ingredient.shown.as_ref().unwrap()[0].damage, 5);
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 3);
    sword.damage = 5;
    assert!(ingredient.test_in(&ctx, &sword));
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(
                ingredient.test_in(&ctx, std::hint::black_box(&sword))
            ));
        }
    });
    assert_eq!(count.calls, 0, "{count:?}");
    assert_eq!(count.bytes, 0);
}
#[test]
fn only_with_component_display_filters_borrowed_presence() {
    let items = Items::empty();
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:only_with_component","component":"minecraft:damage","contents":{"type":"minecraft:composite","contents":[{"type":"minecraft:item","item":"minecraft:apple"},{"type":"minecraft:item_stack","item":{"id":"minecraft:diamond_sword","components":{"minecraft:damage":5}}}]}}}),&items).unwrap();
    let shown = ingredient.shown.unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(&*shown[0].item, "minecraft:diamond_sword");
    assert_eq!(shown[0].damage, 5);
}

#[test]
fn native_registered_component_uses_persistent_nbt_validation_and_equality() {
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .persistent_components
        .push("fixture:registered".into());
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"fixture:registered":{"enabled":true}}}),&items).unwrap();
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&ctx, &stack));
    stack.data_mut().set_other(
        "fixture:registered",
        Some(horizon_core::nbt::Tag::compound([(
            "enabled",
            horizon_core::nbt::Tag::Byte(1),
        )])),
    );
    assert!(ingredient.test_in(&ctx, &stack));
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        assert!(std::hint::black_box(ingredient.test_in(&ctx, &stack)));
    });
    assert_eq!(count.calls, 0, "{count:?}");
    stack.data_mut().set_other(
        "fixture:registered",
        Some(horizon_core::nbt::Tag::compound([(
            "enabled",
            horizon_core::nbt::Tag::Byte(0),
        )])),
    );
    assert!(!ingredient.test_in(&ctx, &stack));
}

#[test]
fn numeric_component_codec_decodes_once_for_matching_and_display() {
    let items = Items::empty();
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5.0,"minecraft:repair_cost":2.0}}),&items).unwrap();
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let shown = &ingredient.shown.as_ref().unwrap()[0];
    assert_eq!(shown.damage, 5);
    assert_eq!(shown.data().unwrap().repair_cost, 2);
    let mut candidate = ItemStack::new("minecraft:diamond_sword", 1);
    candidate.damage = 5;
    candidate.data_mut().repair_cost = 2;
    assert!(ingredient.test_in(&ctx, &candidate));
    candidate.data_mut().repair_cost = 3;
    assert!(!ingredient.test_in(&ctx, &candidate));
}

#[test]
fn implicit_ominous_components_match_without_encoding_or_allocating() {
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:components","items":"minecraft:white_banner","components":{"minecraft:item_name":{"translate":"block.minecraft.ominous_banner"},"minecraft:rarity":"uncommon"}}),&Items::empty()).unwrap();
    let load = context();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:white_banner", 1);
    stack.data_mut().ominous_banner = true;
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        assert!(std::hint::black_box(ingredient.test_in(&ctx, &stack)));
    });
    assert_eq!(count.calls, 0, "{count:?}");
}
