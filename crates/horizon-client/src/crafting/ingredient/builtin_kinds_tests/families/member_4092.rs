use super::*;

#[test]
fn unbreakable_unit_recipe_matches_presence_defaults_and_removal() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:unbreakable":{"ignored":"default"}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let source: Value = serde_json::from_str(
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:stone"],"components":{"minecraft:unbreakable":{"show_in_tooltip":false,"unknown":[1,2]}},"strict":true}],"result":{"id":"minecraft:diamond"}}"#,
    )
    .unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![("fixture:unbreakable".into(), source)],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let matches = |stack: ItemStack| {
        recipes.recipe_matches(
            &context,
            "fixture:unbreakable",
            &[Some(stack), None, None, None],
            2,
        )
    };
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(matches(sword.clone()));
    sword.data_mut().set_other(
        "!minecraft:unbreakable",
        Some(Tag::Compound(Default::default())),
    );
    assert!(!matches(sword.clone()));
    let removal = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"!minecraft:unbreakable":{}}}"#,
        &items,
    );
    assert!(removal.test_in(&context, &sword));
    assert!(!removal.test_in(&context, &ItemStack::new("minecraft:diamond_sword", 1)));
    let mut stone = ItemStack::new("minecraft:stone", 3);
    assert!(!matches(stone.clone()));
    stone.data_mut().set_other(
        "minecraft:unbreakable",
        Some(Tag::compound([(
            "unrecognized",
            Tag::String("live".into()),
        )])),
    );
    assert!(matches(stone.clone()));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":{"ignored":false}}}"#,
        &items,
    );
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:unbreakable"),
        Some(&Tag::Compound(Default::default()))
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
        .set_other("minecraft:custom_data", Some(Tag::Int(1)));
    assert!(!matches(stone.clone()));
    stone.count = 0;
    assert!(!ingredient.test_in(&context, &stone));
}

#[test]
fn unbreakable_unit_recipe_rejects_non_maps_before_publication() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for payload in [r#"true"#, "0", r#""unit""#, "[]", "[{}]"] {
        let mut source: Value = serde_json::from_str(
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":true}}],"result":{"id":"minecraft:diamond"}}"#,
        )
        .unwrap();
        source["ingredients"][0]["components"]["minecraft:unbreakable"] =
            serde_json::from_str(payload).unwrap();
        assert!(Ingredient::parse(&source["ingredients"][0], &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![("fixture:invalid_unbreakable".into(), source)],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty(), "{payload}");
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn rollup_rejection_and_absence_matrix_4092() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":{}}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:unbreakable":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:unbreakable":{}})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([("!minecraft:unbreakable", Tag::Compound(Default::default()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &stack));
    assert!(!positive.test_in(&context, &stack));
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:unbreakable".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:unbreakable"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:unbreakable"), "{error}");
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
