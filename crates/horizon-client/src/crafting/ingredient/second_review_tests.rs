use super::*;
use horizon_core::nbt::Tag;
use horizon_world::loot::PackOverlay;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
fn load() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}
fn resources() -> Items {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap()
}
fn component(items: &Items, id: &str, patch: Value) -> Ingredient {
    let value = serde_json::json!({"type":"neoforge:components","items":id,"components":patch});
    let loaded = recipe(
        items,
        serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[value.clone()],"result":{"id":"minecraft:diamond"}}),
    );
    assert_eq!(
        loaded.crafting.len(),
        1,
        "registered component recipe must load: {value}"
    );
    Ingredient::parse(&value, items).unwrap()
}

fn recipe(items: &Items, json: Value) -> crate::crafting::Recipes {
    let root = tempfile::tempdir().unwrap();
    crate::crafting::Recipes::from_sources(
        root.path(),
        items,
        &PackOverlay::NONE,
        vec![("fixture:second_review".into(), json)],
        &load(),
    )
    .unwrap()
}
#[test]
#[ignore = "requires prepared resources"]
fn registered_stored_removal_only_checks_presence_and_keeps_recipe() {
    let items = resources();
    let ingredient = component(
        &items,
        "minecraft:diamond_sword",
        serde_json::json!({"!minecraft:unbreakable":{}}),
    );
    let loaded = recipe(
        &items,
        serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"!minecraft:unbreakable":{}}}],"result":{"id":"minecraft:diamond"}}),
    );
    assert_eq!(loaded.crafting.len(), 1);
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(ingredient.test_in(&ctx, &stack));
    stack.data_mut().set_other(
        "minecraft:unbreakable",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!ingredient.test_in(&ctx, &stack));
    assert!(!ingredient.shown.as_ref().unwrap()[0].carries_component(
        items.get("minecraft:diamond_sword"),
        "minecraft:unbreakable"
    ));
}
#[test]
#[ignore = "requires prepared resources"]
fn registered_default_component_without_decoder_loads_and_compares_default() {
    let items = resources();
    let ingredient = component(
        &items,
        "minecraft:diamond_sword",
        serde_json::json!({"minecraft:item_model":"minecraft:diamond_sword"}),
    );
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(ingredient.test_in(&ctx, &stack));
    stack.data_mut().set_other(
        "minecraft:item_model",
        Some(Tag::String("minecraft:stone".into())),
    );
    assert!(!ingredient.test_in(&ctx, &stack));
}
#[test]
#[ignore = "requires prepared resources"]
fn modelled_potion_stored_value_matches_same_potion_without_allocating() {
    let items = resources();
    let ingredient = component(
        &items,
        "minecraft:potion",
        serde_json::json!({"minecraft:potion_contents":{"potion":"minecraft:water"}}),
    );
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let same = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
    let other = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:healing");
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(ingredient.test_in(&ctx, &same)));
            assert!(!std::hint::black_box(ingredient.test_in(&ctx, &other)));
        }
    });
    assert_eq!(count.calls, 0, "{count:?}");
    assert_eq!(count.bytes, 0);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .potion
            .as_deref(),
        Some("minecraft:water")
    );
}
#[test]
#[ignore = "requires prepared resources"]
fn raw_stored_component_uses_exact_tag_equality_and_patched_display() {
    let items = resources();
    let ingredient = component(
        &items,
        "minecraft:stone",
        serde_json::json!({"minecraft:custom_data":{"marker":1}}),
    );
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&ctx, &stack));
    stack.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("marker", Tag::Byte(1))])),
    );
    assert!(ingredient.test_in(&ctx, &stack));
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        assert!(std::hint::black_box(ingredient.test_in(&ctx, &stack)))
    });
    assert_eq!(count.calls, 0);
    stack.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("marker", Tag::Byte(2))])),
    );
    assert!(!ingredient.test_in(&ctx, &stack));
    stack.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("marker", Tag::Int(1))])),
    );
    assert!(
        !ingredient.test_in(&ctx, &stack),
        "raw NBT widths are value identity"
    );
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:custom_data"),
        Some(&Tag::compound([("marker", Tag::Byte(1))]))
    );
}
#[test]
fn difference_preserves_component_base_candidates_and_concrete_base_test() {
    let items = Items::empty();
    let json = serde_json::json!({"type":"neoforge:difference","base":{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}},"subtracted":"minecraft:stone"});
    let ingredient = Ingredient::parse(&json, &items).unwrap();
    assert_eq!(ingredient.order, ["minecraft:diamond_sword"]);
    let loaded = recipe(
        &items,
        serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[json],"result":{"id":"minecraft:diamond"}}),
    );
    assert_eq!(loaded.crafting.len(), 1);
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!ingredient.test_in(&ctx, &stack));
    stack.damage = 5;
    assert!(ingredient.test_in(&ctx, &stack));
}
#[test]
fn mod_owned_recipe_type_skips_engine_component_validation() {
    mod_platform::testing::with_recipe_types(&["fixture:owned"], || {
        let loaded = recipe(
            &Items::empty(),
            serde_json::json!({"type":"fixture:owned","ingredients":[{"type":"neoforge:components","items":"minecraft:stone","components":{"!fixture:not_registered":{}}}]}),
        );
        assert_eq!(loaded.by_type["fixture:owned"].len(), 1);
    });
}
#[test]
fn cooking_tests_non_simple_candidates_outside_default_list_in_recipe_order() {
    let items = Items::empty();
    // At default damage 0 this difference lists only stone, but damage 5 sword passes.
    let ingredient = serde_json::json!({"type":"neoforge:difference","base":["minecraft:diamond_sword","minecraft:stone"],"subtracted":{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":0}}});
    let mut items = items;
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:damage":0})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let root = tempfile::tempdir().unwrap();
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let loaded=crate::crafting::Recipes::from_sources(root.path(),&items,&PackOverlay::NONE,vec![
        ("a_complex".into(),serde_json::json!({"type":"minecraft:smelting","ingredient":ingredient,"result":{"id":"minecraft:diamond"},"experience":0.0,"cookingtime":20})),
        ("b_simple".into(),serde_json::json!({"type":"minecraft:smelting","ingredient":"minecraft:diamond_sword","result":{"id":"minecraft:emerald"},"experience":0.0,"cookingtime":20}))],&load).unwrap();
    assert_eq!(loaded.cooking[0].ingredient.order, ["minecraft:stone"]);
    let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
    stack.damage = 5;
    assert_eq!(
        &*loaded
            .cook(&ctx, "minecraft:smelting", &stack)
            .unwrap()
            .result
            .item,
        "minecraft:diamond"
    );
    stack.damage = 0;
    assert_eq!(
        &*loaded
            .cook(&ctx, "minecraft:smelting", &stack)
            .unwrap()
            .result
            .item,
        "minecraft:emerald"
    );
}
#[test]
fn custom_display_empty_tag_loads_with_an_empty_display_list() {
    let ingredient=Ingredient::parse(&serde_json::json!({"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:tag","tag":"#minecraft:missing"}}),&Items::empty()).unwrap();
    assert!(ingredient.shown.as_ref().unwrap().is_empty());
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    assert!(ingredient.test_in(&ctx, &ItemStack::new("minecraft:apple", 1)));
}

#[test]
#[ignore = "requires prepared resources"]
fn modelled_bundle_and_trim_values_compare_complete_fields_without_allocations() {
    let items = resources();
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let bundle = component(
        &items,
        "minecraft:bundle",
        serde_json::json!({"minecraft:bundle_contents":[{"id":"minecraft:stone","count":2}]}),
    );
    let mut same = ItemStack::new("minecraft:bundle", 1);
    same.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 2)];
    let mut other = same.clone();
    other.data_mut().bundle_contents[0].count = 3;
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        assert!(bundle.test_in(&ctx, &same));
        assert!(!bundle.test_in(&ctx, &other));
    });
    assert_eq!(count.calls, 0, "{count:?}");
    assert_eq!(
        bundle.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .bundle_contents[0]
            .count,
        2
    );
    let trim = component(
        &items,
        "minecraft:diamond_chestplate",
        serde_json::json!({"minecraft:trim":{"material":"minecraft:quartz","pattern":"minecraft:spire"}}),
    );
    let mut same = ItemStack::new("minecraft:diamond_chestplate", 1);
    same.data_mut().trim = Some(("minecraft:quartz".into(), "minecraft:spire".into()));
    let mut other = same.clone();
    other.data_mut().trim = Some(("minecraft:quartz".into(), "minecraft:sentry".into()));
    let (_, count) = horizon_threads::allocation_probe::measure(|| {
        assert!(trim.test_in(&ctx, &same));
        assert!(!trim.test_in(&ctx, &other));
    });
    assert_eq!(count.calls, 0, "{count:?}");
}

#[test]
#[ignore = "requires prepared resources"]
fn non_simple_empty_default_list_loads_matches_and_cooks_with_full_candidates() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources"),
    );
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let items = Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
    let json = serde_json::json!({"type":"neoforge:difference","base":"minecraft:diamond_sword","subtracted":{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":0}}});
    let ingredient = Ingredient::parse(&json, &items).unwrap();
    assert!(!ingredient.is_simple());
    assert!(ingredient.order.is_empty());
    assert_eq!(
        ingredient.items,
        std::collections::HashSet::from(["minecraft:diamond_sword".to_owned()])
    );
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!ingredient.test_in(&ctx, &sword));
    sword.damage = 5;
    assert!(ingredient.test_in(&ctx, &sword));
    let recipes = recipe(
        &items,
        serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[json.clone()],"result":{"id":"minecraft:diamond"}}),
    );
    assert_eq!(recipes.crafting.len(), 1);
    assert!(recipes.recipe_matches(&ctx, "fixture:second_review", &[Some(sword.clone())], 1));
    let mut undamaged = sword.clone();
    undamaged.damage = 0;
    assert!(!recipes.recipe_matches(&ctx, "fixture:second_review", &[Some(undamaged)], 1));
    let cooking = recipe(
        &items,
        serde_json::json!({"type":"minecraft:smelting","ingredient":json,"result":{"id":"minecraft:diamond"},"experience":0.0,"cookingtime":20}),
    );
    assert_eq!(cooking.cooking.len(), 1);
    assert_eq!(
        cooking
            .cook(&ctx, "minecraft:smelting", &sword)
            .unwrap()
            .result
            .item
            .as_ref(),
        "minecraft:diamond"
    );
    sword.damage = 0;
    assert!(cooking.cook(&ctx, "minecraft:smelting", &sword).is_none());
}
