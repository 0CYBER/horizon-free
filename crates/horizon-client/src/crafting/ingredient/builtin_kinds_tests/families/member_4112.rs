use super::*;

#[test]
fn note_block_sound_component_recipe_literals_validate_and_match() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    for source in [
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":":unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
    ] {
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:note_block_sound".into(),
                serde_json::from_str(source).unwrap(),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{source}");
        let mut stack = ItemStack::new("minecraft:stick", 1);
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:note_block_sound",
            &[Some(stack.clone())],
            1,
        ));
        for sound in [
            "unregistered/sound",
            "minecraft:unregistered/sound",
            ":unregistered/sound",
        ] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(recipes.recipe_matches(
                &context,
                "fixture:note_block_sound",
                &[Some(stack.clone())],
                1,
            ));
        }
        for sound in [
            "minecraft:other",
            "fixture:unregistered/sound",
            "Minecraft:unregistered/sound",
            "",
        ] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(!recipes.recipe_matches(
                &context,
                "fixture:note_block_sound",
                &[Some(stack.clone())],
                1,
            ));
        }
    }
    for source in [
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":42}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":{}}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"Minecraft:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:Sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"..:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"bad namespace:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"a:b:c"}}],"result":{"id":"minecraft:diamond"}}"#,
    ] {
        let value: Value = serde_json::from_str(source).unwrap();
        let ingredient = &value["ingredients"][0];
        assert!(Ingredient::parse(ingredient, &items).is_none(), "{source}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(ingredient["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:note_block_sound:"),
            "{error}"
        );
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![("fixture:invalid_note_block_sound".into(), value)],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty(), "{source}");
        assert!(
            recipes.by_type.values().all(|recipes| recipes.is_empty()),
            "{source}"
        );
    }
}

#[test]
fn note_block_sound_component_defaults_removal_and_strict_matching() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:note_block_sound":":unregistered/sound"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"unregistered/sound"},"strict":true}"#,
        &items,
    );
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:note_block_sound"),
        Some(&Tag::String("minecraft:unregistered/sound".into())),
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(ingredient.test_in(&context, &stack));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for sound in [
        "unregistered/sound",
        "minecraft:unregistered/sound",
        ":unregistered/sound",
    ] {
        stack.data_mut().set_loot_component(
            "minecraft:note_block_sound",
            &Some(Tag::String(sound.into())),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
    }
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_name", &Some(Tag::String("Named".into())));
    assert!(!ingredient.test_in(&context, &stack));
    stack = ItemStack::new("minecraft:stick", 1);
    let removal = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"!minecraft:note_block_sound":{}}}"#,
        &items,
    );
    assert!(!removal.test_in(&context, &stack));
    let patch = Tag::compound([(
        "!minecraft:note_block_sound",
        Tag::Compound(Default::default()),
    )]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(removal.test_in(&context, &stack));
    stack = ItemStack::new("minecraft:stick", 1);

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":""}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":":"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:"}}"#,
    ] {
        let empty = parse(source, &items);
        assert!(!empty.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
        for sound in ["", ":", "minecraft:"] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(empty.test_in(&context, &stack));
        }
        for value in [
            Tag::Int(0),
            Tag::String("fixture:".into()),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
            Tag::String(horizon_core::nbt::JavaString(vec![0xdc00])),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd83d, 0xde00])),
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:note_block_sound", &Some(value));
            assert!(!empty.test_in(&context, &stack));
        }
    }
}

#[test]
fn rollup_rejection_and_absence_matrix_4112() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:note_block_sound":"fixture:sound"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:note_block_sound":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:note_block_sound":"fixture:sound"})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([(
        "!minecraft:note_block_sound",
        Tag::Compound(Default::default()),
    )]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &stack));
    assert!(!positive.test_in(&context, &stack));
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:note_block_sound".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:note_block_sound"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:note_block_sound":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:note_block_sound"), "{error}");
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
