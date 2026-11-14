use super::*;

#[test]
fn intangible_projectile_recipe_decodes_maps_and_matches_borrowed_units() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:arrow".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:intangible_projectile":{"default_field":false}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let root = tempfile::tempdir().unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:intangible_projectile".into(),
            serde_json::from_str(
                r#"{"type":"minecraft:smelting","ingredient":{"type":"neoforge:components","items":["minecraft:arrow","minecraft:stone"],"components":{"minecraft:intangible_projectile":{"ignored":[1,2]}},"strict":true},"result":{"id":"minecraft:diamond"},"experience":0.0,"cookingtime":20}"#,
            )
            .unwrap(),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.cooking.len(), 1);
    let ingredient = &recipes.cooking[0].ingredient;
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:intangible_projectile"),
        Some(&Tag::Compound(Default::default()))
    );
    let mut arrow = ItemStack::new("minecraft:arrow", 1);
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &arrow)
            .is_some()
    );
    let mut stone = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&context, &stone));
    stone.data_mut().set_other(
        "minecraft:intangible_projectile",
        Some(Tag::compound([(
            "different",
            Tag::String("ignored".into()),
        )])),
    );
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &stone)
            .is_some()
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stone)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stone
        .data_mut()
        .set_other("minecraft:intangible_projectile", Some(Tag::Byte(1)));
    assert!(!ingredient.test_in(&context, &stone));
    let absent = parse(
        r#"{"type":"neoforge:components","items":["minecraft:arrow","minecraft:stone"],"components":{"!minecraft:intangible_projectile":{}}}"#,
        &items,
    );
    assert!(!absent.test_in(&context, &arrow));
    let patch = Tag::compound([(
        "!minecraft:intangible_projectile",
        Tag::Compound(Default::default()),
    )]);
    (_, arrow.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(absent.test_in(&context, &arrow));
    assert!(!ingredient.test_in(&context, &arrow));
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &arrow)
            .is_none()
    );
    assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
}

#[test]
fn intangible_projectile_invalid_payload_rejects_whole_recipe_with_component_error() {
    let items = Items::empty();
    let root = tempfile::tempdir().unwrap();
    let load = load();
    for value in [
        serde_json::json!(true),
        serde_json::json!(0),
        serde_json::json!("unit"),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:intangible_projectile":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        assert!(
            super::components::validate(&rule, &items)
                .unwrap_err()
                .contains("component minecraft:intangible_projectile:")
        );
        let ingredient = serde_json::json!({
            "type":"neoforge:components",
            "items":"minecraft:arrow",
            "components":patch
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_intangible_projectile".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless",
                    "ingredients":[ingredient],
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
fn rollup_boundary_and_absence_4100() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for inherited in [false, true] {
        let mut items = Items::empty();
        let defaults = crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:intangible_projectile":{}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        if inherited {
            items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
                "minecraft:arrow".into(),
                defaults.clone(),
            )]));
        }
        let positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:arrow","components":{"minecraft:intangible_projectile":{}}}"#,
            &items,
        );
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:arrow","components":{"!minecraft:intangible_projectile":{}}}"#,
            &items,
        );
        let mut stack = ItemStack::new("minecraft:arrow", 1);
        assert_eq!(positive.test_in(&context, &stack), inherited);
        assert_eq!(absent.test_in(&context, &stack), !inherited);
        (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(defaults)));
        assert!(positive.test_in(&context, &stack));
        assert!(!absent.test_in(&context, &stack));
        let removal = Tag::compound([(
            "!minecraft:intangible_projectile",
            Tag::Compound(Default::default()),
        )]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
        assert!(absent.test_in(&context, &stack));
        assert!(!positive.test_in(&context, &stack));
    }
    let items = Items::empty();
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let registry = horizon_command::Registries::default();
        let error = horizon_command::item::decoded_component_value(
            "minecraft:intangible_projectile",
            &tag,
            &registry,
        )
        .unwrap_err();
        assert!(!error.is_empty());
        let rule = IngredientRule::Components {
            patch: [("minecraft:intangible_projectile".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:intangible_projectile"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:arrow","components":{"minecraft:intangible_projectile":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:intangible_projectile"), "{error}");
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        let root = tempfile::tempdir().unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_intangible_projectile".into(),
                    recipe(ingredient),
                ),
                (
                    "fixture:valid".into(),
                    recipe(serde_json::json!("minecraft:arrow")),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:invalid_intangible_projectile",
            &[Some(ItemStack::new("minecraft:arrow", 1))],
            1
        ));
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid",
            &[Some(ItemStack::new("minecraft:arrow", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:arrow", 1))], 1)
                .is_some()
        );
    }
}
