use super::*;
fn horse_variant_recipes(
    items: &Items,
    patch: serde_json::Value,
    strict: bool,
) -> crate::crafting::Recipes {
    let ingredient = serde_json::json!({
        "type": "neoforge:components",
        "items": "minecraft:horse_spawn_egg",
        "components": patch,
        "strict": strict
    });
    let root = tempfile::tempdir().unwrap();
    crate::crafting::Recipes::from_sources(
        root.path(),
        items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![
            (
                "fixture:horse_shapeless".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient.clone()],
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:horse_shaped".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["A"],
                    "key": {"A": ingredient.clone()},
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:horse_smelting".into(),
                serde_json::json!({
                    "type": "minecraft:smelting",
                    "ingredient": ingredient,
                    "result": {"id": "minecraft:diamond"},
                    "experience": 0.0,
                    "cookingtime": 20
                }),
            ),
        ],
        &load(),
    )
    .unwrap()
}

#[test]
fn horse_variant_recipe_codec_matches_exact_names_without_candidate_allocations() {
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .persistent_components
        .push("minecraft:custom_data".into());
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (name, different) in [
        ("white", "black"),
        ("creamy", "white"),
        ("chestnut", "white"),
        ("brown", "white"),
        ("black", "white"),
        ("gray", "white"),
        ("dark_brown", "white"),
    ] {
        // An unqualified component key normalizes, but the enum payload does not.
        let recipes =
            horse_variant_recipes(&items, serde_json::json!({"horse/variant": name}), false);
        assert_eq!(recipes.crafting.len(), 2);
        assert_eq!(recipes.cooking.len(), 1);
        for (_, recipe) in &recipes.crafting {
            let ingredient = match recipe {
                crate::crafting::Recipe::Shapeless { ingredients, .. } => &ingredients[0],
                crate::crafting::Recipe::Shaped { cells, .. } => cells[0].as_ref().unwrap(),
                _ => panic!("literal crafting recipe expected"),
            };
            assert_eq!(
                ingredient.shown.as_ref().unwrap()[0]
                    .data()
                    .unwrap()
                    .other("minecraft:horse/variant"),
                Some(&Tag::String(name.into()))
            );
            assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:horse_spawn_egg", 1)));
            let mut stack = ItemStack::new("minecraft:horse_spawn_egg", 7);
            stack
                .data_mut()
                .set_other("minecraft:horse/variant", Some(Tag::String(name.into())));
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..100 {
                    assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
            for value in [
                Tag::String(different.into()),
                Tag::Int(if name == "white" { 0 } else { 1 }),
                Tag::String(horizon_core::nbt::JavaString(vec![114, 101, 100, 0xd800])),
            ] {
                stack
                    .data_mut()
                    .set_other("minecraft:horse/variant", Some(value));
                assert!(!ingredient.test_in(&context, &stack));
            }
            stack
                .data_mut()
                .set_other("minecraft:horse/variant", Some(Tag::String(name.into())));
            stack.count = 0;
            assert!(!ingredient.test_in(&context, &stack));
        }
        let mut egg = ItemStack::new("minecraft:horse_spawn_egg", 1);
        egg.data_mut()
            .set_other("minecraft:horse/variant", Some(Tag::String(name.into())));
        assert_eq!(
            recipes
                .craft(&context, &[Some(egg.clone())], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_some());
        egg.data_mut().set_other(
            "minecraft:horse/variant",
            Some(Tag::String(different.into())),
        );
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_none());
    }
}

#[test]
fn horse_variant_recipe_defaults_removal_and_strictness_preserve_presence() {
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .persistent_components
        .push("minecraft:custom_data".into());
    // Synthetic pack prototype: vanilla horse spawn eggs have no horse/variant.
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:horse_spawn_egg".into(),
        match Tag::compound([("minecraft:horse/variant", Tag::String("white".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let exact = horse_variant_recipes(
        &items,
        serde_json::json!({"minecraft:horse/variant": "white"}),
        true,
    );
    let loose = horse_variant_recipes(
        &items,
        serde_json::json!({"minecraft:horse/variant": "white"}),
        false,
    );
    let omitted = horse_variant_recipes(&items, serde_json::json!({}), true);
    // Removal payloads require registration, not a successful variant decode.
    let removal = horse_variant_recipes(
        &items,
        serde_json::json!({"!minecraft:horse/variant": 123}),
        true,
    );
    assert_eq!(removal.crafting.len(), 2);
    let mut egg = ItemStack::new("minecraft:horse_spawn_egg", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    egg.data_mut()
        .set_loot_component("minecraft:horse/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:horse/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:horse/variant",
        &Some(Tag::String("black".into())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
    let absent = horse_variant_recipes(
        &Items::empty(),
        serde_json::json!({"!minecraft:horse/variant": {}}),
        true,
    );
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:horse_spawn_egg", 1))],
                1
            )
            .is_some()
    );
}

#[test]
fn horse_variant_invalid_codec_payloads_omit_whole_recipes() {
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .persistent_components
        .push("minecraft:custom_data".into());
    for (payload, diagnostic) in [
        (serde_json::json!(0), "Not a string"),
        (serde_json::json!(1), "Not a string"),
        (serde_json::json!(1.0), "Not a string"),
        (serde_json::json!(false), "Not a string"),
        (serde_json::json!({}), "Not a string"),
        (serde_json::json!([]), "Not a string"),
        (serde_json::json!("WHITE"), "Unknown element name:WHITE"),
        (serde_json::json!("arctic"), "Unknown element name:arctic"),
        (
            serde_json::json!("minecraft:white"),
            "Unknown element name:minecraft:white",
        ),
        (serde_json::json!(""), "Unknown element name:"),
    ] {
        let patch = serde_json::json!({"minecraft:horse/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        assert_eq!(
            components::validate(&rule, &items),
            Err(format!("component minecraft:horse/variant: {diagnostic}"))
        );
        let ingredient = serde_json::json!({
            "type": "neoforge:components",
            "items": "minecraft:horse_spawn_egg",
            "components": patch.clone()
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        super::decoder_contracts::assert_malformed_decoder_reload_keeps_sibling(&items, patch);
    }
}
