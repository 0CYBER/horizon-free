use super::*;

fn parrot_variant_recipes(
    items: &Items,
    patch: serde_json::Value,
    strict: bool,
) -> crate::crafting::Recipes {
    let ingredient = serde_json::json!({
        "type": "neoforge:components",
        "items": "minecraft:parrot_spawn_egg",
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
                "fixture:parrot_shapeless".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient.clone()],
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:parrot_shaped".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["A"],
                    "key": {"A": ingredient.clone()},
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:parrot_smelting".into(),
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
fn parrot_variant_recipe_codec_presence_strictness_and_borrowed_matching() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    // Parrot.Variant.CODEC names, independently taken from the pinned enum.
    for name in ["red_blue", "blue", "green", "yellow_blue", "gray"] {
        let recipes =
            parrot_variant_recipes(&items, serde_json::json!({"parrot/variant": name}), false);
        assert_eq!(recipes.crafting.len(), 2);
        assert_eq!(recipes.cooking.len(), 1);
        let mut egg = ItemStack::new("minecraft:parrot_spawn_egg", 7);
        // Items.registerSpawnEgg -> Properties.spawnEgg only adds ENTITY_DATA.
        // Parrot.Variant.DEFAULT is a mob default, not an item prototype value.
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        egg.data_mut()
            .set_other("minecraft:parrot/variant", Some(Tag::String(name.into())));
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
                    .other("minecraft:parrot/variant"),
                Some(&Tag::String(name.into()))
            );
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..100 {
                    assert!(std::hint::black_box(ingredient.test_in(&context, &egg)));
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
        }
        assert_eq!(
            recipes
                .craft(&context, &[Some(egg.clone())], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_some());
        for value in [
            Tag::String(if name == "gray" { "blue" } else { "gray" }.into()),
            Tag::Int(0),
            Tag::String(horizon_core::nbt::JavaString(vec![
                98, 108, 117, 101, 0xd800,
            ])),
        ] {
            egg.data_mut()
                .set_other("minecraft:parrot/variant", Some(value));
            assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
            assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_none());
        }
    }
    for payload in [
        serde_json::json!(0),
        serde_json::json!(4),
        serde_json::json!(1.0),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!("RED_BLUE"),
        serde_json::json!("red"),
        serde_json::json!("minecraft:blue"),
        serde_json::json!(""),
    ] {
        let patch = serde_json::json!({"minecraft:parrot/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.starts_with("component minecraft:parrot/variant:"));
        let invalid = parrot_variant_recipes(&items, patch, false);
        assert!(invalid.crafting.is_empty());
        assert!(invalid.cooking.is_empty());
        assert!(invalid.by_type.values().all(|recipes| recipes.is_empty()));
    }
    // Synthetic supplied prototype exercises inherited values; vanilla supplies none.
    let mut prototypes = Items::empty();
    prototypes.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:parrot_spawn_egg".into(),
        match Tag::compound([("minecraft:parrot/variant", Tag::String("green".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    for (items, inherited) in [(&items, false), (&prototypes, true)] {
        let exact = parrot_variant_recipes(
            items,
            serde_json::json!({"minecraft:parrot/variant": "green"}),
            true,
        );
        let loose = parrot_variant_recipes(
            items,
            serde_json::json!({"minecraft:parrot/variant": "green"}),
            false,
        );
        let absent = parrot_variant_recipes(
            items,
            serde_json::json!({"!minecraft:parrot/variant": 123}),
            true,
        );
        let omitted = parrot_variant_recipes(items, serde_json::json!({}), true);
        let mut egg = ItemStack::new("minecraft:parrot_spawn_egg", 1);
        assert_eq!(
            exact.craft(&context, &[Some(egg.clone())], 1).is_some(),
            inherited
        );
        assert_eq!(
            absent.craft(&context, &[Some(egg.clone())], 1).is_some(),
            !inherited
        );
        assert_eq!(
            omitted.craft(&context, &[Some(egg.clone())], 1).is_some(),
            !inherited
        );
        egg.data_mut().set_other(
            "minecraft:parrot/variant",
            Some(Tag::String("green".into())),
        );
        egg.data_mut()
            .set_other("minecraft:custom_data", Some(Tag::Int(3)));
        assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
        assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
        let removed = Tag::compound([(
            "!minecraft:parrot/variant",
            Tag::compound([] as [(&str, Tag); 0]),
        )]);
        (_, egg.extra) = crate::item_data::decode_components(Some(&removed));
        assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(absent.craft(&context, &[Some(egg.clone())], 1).is_some());
        egg.data_mut().set_loot_component(
            "minecraft:parrot/variant",
            &Some(Tag::String("blue".into())),
        );
        assert!(absent.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_none());
        egg.data_mut().set_other(
            "minecraft:parrot/variant",
            Some(Tag::String("green".into())),
        );
        egg.count = 0;
        assert!(exact.craft(&context, &[Some(egg)], 1).is_none());
    }
}

#[test]
fn rollup_rejection_and_absence_matrix_4222() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:parrot/variant":"green"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:parrot/variant":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:parrot/variant":"green"})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([(
        "!minecraft:parrot/variant",
        Tag::Compound(Default::default()),
    )]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &stack));
    assert!(!positive.test_in(&context, &stack));
    for malformed in [
        Tag::End,
        Tag::Byte(0),
        Tag::List(vec![]),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        Tag::String("unknown".into()),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:parrot/variant", &Some(malformed.clone()));
        assert!(!positive.test_in(&context, &stack));
        let rule = IngredientRule::Components {
            patch: [("minecraft:parrot/variant".into(), malformed)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        assert!(
            components::validate(&rule, &items)
                .unwrap_err()
                .contains("minecraft:parrot/variant")
        );
    }
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:parrot/variant".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:parrot/variant"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(4),
        serde_json::json!(1.0),
        serde_json::json!({}),
        serde_json::json!("RED_BLUE"),
        serde_json::json!("red"),
        serde_json::json!("minecraft:blue"),
        serde_json::json!(""),
        serde_json::json!(false),
        serde_json::json!([]),
        serde_json::json!("unknown"),
        serde_json::json!("Minecraft:green"),
        serde_json::json!("green".to_uppercase()),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:parrot/variant":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:parrot/variant"), "{error}");
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        let root = tempfile::tempdir().unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid".into(), recipe(ingredient)),
                (
                    "fixture:valid".into(),
                    recipe(serde_json::json!("minecraft:stone")),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stone", 1))], 1)
                .is_some()
        );
    }
}

#[test]
fn inherited_setter_removal_4222() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for inherited in [false, true] {
        let mut items = Items::empty();
        if inherited {
            let Tag::Compound(defaults) =
                Tag::compound([("minecraft:parrot/variant", Tag::String("green".into()))])
            else {
                unreachable!()
            };
            items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
                "minecraft:stone".into(),
                defaults,
            )]));
        }
        let positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:parrot/variant":"green"}}"#,
            &items,
        );
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:parrot/variant":{}}}"#,
            &items,
        );
        let mut stack = ItemStack::new("minecraft:stone", 1);
        assert_eq!(positive.test_in(&context, &stack), inherited);
        assert_eq!(absent.test_in(&context, &stack), !inherited);
        stack.data_mut().set_loot_component(
            "minecraft:parrot/variant",
            &Some(Tag::String("green".into())),
        );
        assert!(positive.test_in(&context, &stack));
        stack
            .data_mut()
            .set_loot_component("minecraft:parrot/variant", &None);
        stack.data_mut().set_loot_component(
            "!minecraft:parrot/variant",
            &Some(Tag::Compound(Default::default())),
        );
        assert!(absent.test_in(&context, &stack));
        assert!(!positive.test_in(&context, &stack));
    }
}
