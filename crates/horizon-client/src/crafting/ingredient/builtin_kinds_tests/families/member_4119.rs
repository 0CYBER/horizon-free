use super::*;

#[test]
fn base_color_recipe_compiles_and_matches_typed_shields() {
    let root = std::env::temp_dir().join(format!("horizon-base-color-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let sources = [
        (
            "fixture:base_color",
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:shield","components":{"minecraft:base_color":"red"},"strict":true}],"result":"minecraft:diamond"}"#,
        ),
        (
            "fixture:invalid_color",
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:shield","components":{"minecraft:base_color":"RED"}}],"result":"minecraft:diamond"}"#,
        ),
        (
            "fixture:numeric_color",
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:shield","components":{"minecraft:base_color":14}}],"result":"minecraft:diamond"}"#,
        ),
    ]
    .into_iter()
    .map(|(id, source)| (id.into(), serde_json::from_str(source).unwrap()))
    .collect();
    let recipes = crate::crafting::Recipes::from_sources(
        &root,
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    assert!(recipes.recipe("fixture:base_color").is_some());
    assert!(recipes.recipe("fixture:invalid_color").is_none());
    assert!(recipes.recipe("fixture:numeric_color").is_none());

    let mut shield = ItemStack::new("minecraft:shield", 1);
    assert!(recipes.find(&context, &[Some(shield.clone())], 1).is_none());
    // A live shield stores this in StackData, not in its `other` NBT fields.
    shield.data_mut().base_color = Some("red".into());
    assert!(shield.data().unwrap().other.is_empty());
    let (id, result) = recipes.find(&context, &[Some(shield.clone())], 1).unwrap();
    assert_eq!(id, "fixture:base_color");
    assert_eq!(result.item.as_ref(), "minecraft:diamond");
    shield.data_mut().base_color = Some("blue".into());
    assert!(recipes.find(&context, &[Some(shield.clone())], 1).is_none());
    shield.data_mut().base_color = Some("red".into());
    shield.count = 0;
    assert!(recipes.find(&context, &[Some(shield)], 1).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn base_color_components_preserve_defaults_removals_and_banner_absence() {
    let mut items = Items::empty();
    // An explicit resource prototype exercises default inheritance independently
    // of vanilla's undyed shield, whose prototype has no base_color.
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:shield".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:base_color":"red"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let red = parse(
        r#"{"type":"neoforge:components","items":"minecraft:shield","components":{"minecraft:base_color":"red"},"strict":true}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":["minecraft:shield","minecraft:red_banner","minecraft:white_banner"],"components":{"!minecraft:base_color":{}}}"#,
        &items,
    );
    let shown = &red.shown.as_ref().unwrap()[0];
    assert_eq!(shown.data().unwrap().base_color.as_deref(), Some("red"));
    assert!(shown.data().unwrap().other.is_empty());
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut shield = ItemStack::new("minecraft:shield", 1);
    assert!(red.test_in(&context, &shield));
    assert!(!absent.test_in(&context, &shield));
    shield.data_mut().base_color = Some("blue".into());
    assert!(!red.test_in(&context, &shield));
    let removal = Tag::compound([("!minecraft:base_color", Tag::Compound(Default::default()))]);
    (_, shield.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(!red.test_in(&context, &shield));
    assert!(absent.test_in(&context, &shield));

    let no_defaults = Items::empty();
    let white = parse(
        r#"{"type":"neoforge:components","items":["minecraft:shield","minecraft:white_banner"],"components":{"minecraft:base_color":"white"}}"#,
        &no_defaults,
    );
    let undyed = ItemStack::new("minecraft:shield", 1);
    assert!(!white.test_in(&context, &undyed));
    for item in ["minecraft:red_banner", "minecraft:white_banner"] {
        let mut banner = ItemStack::new(item, 1);
        assert!(absent.test_in(&context, &banner));
        assert!(!white.test_in(&context, &banner));
        banner.data_mut().ominous_banner = true;
        // Horizon's implicit ominous layers/name/rarity never imply base_color.
        assert!(absent.test_in(&context, &banner));
        assert!(!white.test_in(&context, &banner));
    }
}

#[test]
fn base_color_component_errors_name_the_component() {
    let items = Items::empty();
    for value in [
        serde_json::json!("unknown"),
        serde_json::json!("Light_Blue"),
        serde_json::json!(14),
        serde_json::json!(true),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        let patch = crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:base_color":value})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let rule = IngredientRule::Components {
            patch,
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.starts_with("component minecraft:base_color:"),
            "{error}"
        );
    }
}

#[test]
fn rollup_rejection_and_absence_matrix_4119() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:base_color":"red"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:base_color":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:base_color":"red"})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([("!minecraft:base_color", Tag::Compound(Default::default()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &stack));
    assert!(!positive.test_in(&context, &stack));
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:base_color".into(), tag)].into_iter().collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:base_color"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:base_color":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:base_color"), "{error}");
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
