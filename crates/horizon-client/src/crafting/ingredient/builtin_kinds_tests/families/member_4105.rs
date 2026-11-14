use super::*;

#[test]
fn waxed_recipe_decodes_unit_maps_and_matches_borrowed_values() {
    let items = Items::empty();
    let source = r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{"ignored":{"nested":true}}}}"#;
    let ingredient = parse(source, &items);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:waxed"),
        Some(&Tag::compound([] as [(&str, Tag); 0]))
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:waxed".into(),
            serde_json::from_str(
                r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{"unknown":42}}}],"result":{"id":"minecraft:diamond"}}"#,
            )
            .unwrap(),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack.clone()), None, None, None], 2)
            .is_none()
    );
    for payload in [
        Tag::compound([] as [(&str, Tag); 0]),
        Tag::compound([("different", Tag::List(vec![Tag::Int(7)]))]),
    ] {
        stack.data_mut().set_other("minecraft:waxed", Some(payload));
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        assert_eq!(
            recipes
                .craft(&context, &[Some(stack.clone()), None, None, None], 2)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
    }
    stack
        .data_mut()
        .set_other("minecraft:waxed", Some(Tag::Byte(1)));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack.clone()), None, None, None], 2)
            .is_none()
    );
    stack.data_mut().set_other("minecraft:waxed", None);
    stack.data_mut().set_other(
        "!minecraft:waxed",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack), None, None, None], 2)
            .is_none()
    );
}

#[test]
fn waxed_defaults_and_removal_preserve_presence() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        horizon_core::nbt::Compound::from([(
            "minecraft:waxed".into(),
            Tag::compound([("ignored", Tag::Int(9))]),
        )]),
    )]));
    let present = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{}},"strict":true}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:waxed":{}}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(present.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &stack));
    stack.data_mut().set_other(
        "!minecraft:waxed",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!present.test_in(&context, &stack));
    assert!(absent.test_in(&context, &stack));
}

#[test]
fn waxed_malformed_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for payload in [
        serde_json::json!(true),
        serde_json::json!(0),
        serde_json::json!("unit"),
        serde_json::json!([]),
    ] {
        let patch = crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:waxed":payload})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let error = super::components::validate(
            &IngredientRule::Components {
                patch,
                removed: vec![],
                strict: false,
            },
            &items,
        )
        .unwrap_err();
        assert!(error.contains("minecraft:waxed"), "{error}");
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:stone",
            "components":{"minecraft:waxed":payload}
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_waxed".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn rollup_boundary_and_absence_4105() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for inherited in [false, true] {
        let mut items = Items::empty();
        let defaults = crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:waxed":{}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        if inherited {
            items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
                "minecraft:map".into(),
                defaults.clone(),
            )]));
        }
        let positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:map","components":{"minecraft:waxed":{}}}"#,
            &items,
        );
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:map","components":{"!minecraft:waxed":{}}}"#,
            &items,
        );
        let mut stack = ItemStack::new("minecraft:map", 1);
        assert_eq!(positive.test_in(&context, &stack), inherited);
        assert_eq!(absent.test_in(&context, &stack), !inherited);
        (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(defaults)));
        assert!(positive.test_in(&context, &stack));
        assert!(!absent.test_in(&context, &stack));
        let removal = Tag::compound([("!minecraft:waxed", Tag::Compound(Default::default()))]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
        assert!(absent.test_in(&context, &stack));
        assert!(!positive.test_in(&context, &stack));
    }
    let items = Items::empty();
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let registry = horizon_command::Registries::default();
        let error =
            horizon_command::item::decoded_component_value("minecraft:waxed", &tag, &registry)
                .unwrap_err();
        assert!(!error.is_empty());
        let rule = IngredientRule::Components {
            patch: [("minecraft:waxed".into(), tag)].into_iter().collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:waxed"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:map","components":{"minecraft:waxed":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:waxed"), "{error}");
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        let root = tempfile::tempdir().unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid_waxed".into(), recipe(ingredient)),
                (
                    "fixture:valid".into(),
                    recipe(serde_json::json!("minecraft:map")),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:invalid_waxed",
            &[Some(ItemStack::new("minecraft:map", 1))],
            1
        ));
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid",
            &[Some(ItemStack::new("minecraft:map", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:map", 1))], 1)
                .is_some()
        );
    }
}
