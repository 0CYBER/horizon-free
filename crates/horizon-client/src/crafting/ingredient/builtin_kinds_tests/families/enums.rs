use super::*;

fn fox_variant_recipes(
    items: &Items,
    patch: serde_json::Value,
    strict: bool,
) -> crate::crafting::Recipes {
    let ingredient = serde_json::json!({
        "type": "neoforge:components",
        "items": "minecraft:fox_spawn_egg",
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
                "fixture:fox_shapeless".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient.clone()],
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:fox_shaped".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["A"],
                    "key": {"A": ingredient.clone()},
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:fox_smelting".into(),
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
fn fox_variant_recipe_codec_matches_exact_names_without_candidate_allocations() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (name, different) in [("red", "snow"), ("snow", "red")] {
        // An unqualified component key normalizes, but the enum payload does not.
        let recipes = fox_variant_recipes(&items, serde_json::json!({"fox/variant": name}), false);
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
                    .other("minecraft:fox/variant"),
                Some(&Tag::String(name.into()))
            );
            assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:fox_spawn_egg", 1)));
            let mut stack = ItemStack::new("minecraft:fox_spawn_egg", 7);
            stack
                .data_mut()
                .set_other("minecraft:fox/variant", Some(Tag::String(name.into())));
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..100 {
                    assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
            for value in [
                Tag::String(different.into()),
                Tag::Int(if name == "red" { 0 } else { 1 }),
                Tag::String(horizon_core::nbt::JavaString(vec![114, 101, 100, 0xd800])),
            ] {
                stack
                    .data_mut()
                    .set_other("minecraft:fox/variant", Some(value));
                assert!(!ingredient.test_in(&context, &stack));
            }
            stack
                .data_mut()
                .set_other("minecraft:fox/variant", Some(Tag::String(name.into())));
            stack.count = 0;
            assert!(!ingredient.test_in(&context, &stack));
        }
        let mut egg = ItemStack::new("minecraft:fox_spawn_egg", 1);
        egg.data_mut()
            .set_other("minecraft:fox/variant", Some(Tag::String(name.into())));
        assert_eq!(
            recipes
                .craft(&context, &[Some(egg.clone())], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_some());
        egg.data_mut()
            .set_other("minecraft:fox/variant", Some(Tag::String(different.into())));
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_none());
    }
}

#[test]
fn fox_variant_recipe_defaults_removal_and_strictness_preserve_presence() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:fox_spawn_egg".into(),
        match Tag::compound([("minecraft:fox/variant", Tag::String("red".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let exact = fox_variant_recipes(
        &items,
        serde_json::json!({"minecraft:fox/variant": "red"}),
        true,
    );
    let loose = fox_variant_recipes(
        &items,
        serde_json::json!({"minecraft:fox/variant": "red"}),
        false,
    );
    let omitted = fox_variant_recipes(&items, serde_json::json!({}), true);
    // Removal payloads require registration, not a successful variant decode.
    let removal = fox_variant_recipes(
        &items,
        serde_json::json!({"!minecraft:fox/variant": 123}),
        true,
    );
    assert_eq!(removal.crafting.len(), 2);
    let mut egg = ItemStack::new("minecraft:fox_spawn_egg", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    egg.data_mut()
        .set_loot_component("minecraft:fox/variant", &Some(Tag::String("red".into())));
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut()
        .set_loot_component("minecraft:fox/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:fox/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut()
        .set_loot_component("minecraft:fox/variant", &Some(Tag::String("snow".into())));
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
    let absent = fox_variant_recipes(
        &Items::empty(),
        serde_json::json!({"!minecraft:fox/variant": {}}),
        true,
    );
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:fox_spawn_egg", 1))],
                1
            )
            .is_some()
    );
}

#[test]
fn fox_variant_invalid_codec_payloads_omit_whole_recipes() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:fox/variant".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:fox/variant"), "{error}");
    }
    for (payload, diagnostic) in [
        (serde_json::json!(null), ""),
        (serde_json::json!(0), "Not a string"),
        (serde_json::json!(1), "Not a string"),
        (serde_json::json!(1.0), "Not a string"),
        (serde_json::json!(false), "Not a string"),
        (serde_json::json!({}), "Not a string"),
        (serde_json::json!([]), "Not a string"),
        (serde_json::json!("RED"), "Unknown element name:RED"),
        (serde_json::json!("arctic"), "Unknown element name:arctic"),
        (
            serde_json::json!("minecraft:red"),
            "Unknown element name:minecraft:red",
        ),
        (serde_json::json!(""), "Unknown element name:"),
    ] {
        let patch = serde_json::json!({"minecraft:fox/variant": payload});
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:fox_spawn_egg", "components":patch.clone()
        });
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:fox/variant"), "{error}");
        if !diagnostic.is_empty() {
            assert!(error.contains(diagnostic), "{error}");
        }
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        let reload = crate::crafting::Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid".into(), recipe(ingredient.clone())),
                (
                    "fixture:valid".into(),
                    recipe(serde_json::json!("minecraft:stick")),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(reload.crafting.len(), 1);
        assert!(
            reload
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .is_some()
        );
        assert!(
            reload
                .craft(
                    &context,
                    &[Some(ItemStack::new("minecraft:fox_spawn_egg", 1))],
                    1
                )
                .is_none()
        );
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = fox_variant_recipes(&items, patch, false);
        assert!(recipes.crafting.is_empty());
        assert!(recipes.cooking.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}
