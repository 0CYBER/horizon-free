use super::*;

#[test]
fn tooltip_style_recipe_decodes_identifiers_and_matches_borrowed_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:tooltip_style":":style"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    for (value, valid) in [
        (serde_json::json!("style"), true),
        (serde_json::json!(":style"), true),
        (serde_json::json!("fixture:"), true),
        (serde_json::json!(""), true),
        (serde_json::json!(":"), true),
        (serde_json::json!("UPPER"), false),
        (serde_json::json!("..:style"), false),
        (serde_json::json!("a:b:c"), false),
        (serde_json::json!(3), false),
    ] {
        let source = serde_json::json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"neoforge:components","items":"minecraft:stone",
                "components":{"minecraft:tooltip_style":value}}],
            "result":{"id":"minecraft:diamond"}
        });
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![("fixture:tooltip_style".into(), source)],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), usize::from(valid), "{value}");
        if !valid {
            continue;
        }
        let crate::crafting::Recipe::Shapeless { ingredients, .. } = &recipes.crafting[0].1 else {
            panic!("expected shapeless recipe");
        };
        let ingredient = &ingredients[0];
        let mut stack = ItemStack::new("minecraft:stone", 1);
        assert_eq!(
            ingredient.test_in(&context, &stack),
            value == "style" || value == ":style"
        );
        stack.data_mut().set_other(
            "minecraft:tooltip_style",
            Some(crate::crafting::json_tag(&value)),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0);
        assert_eq!(allocations.bytes, 0);
        stack.data_mut().set_other(
            "minecraft:tooltip_style",
            Some(Tag::String("fixture:different".into())),
        );
        assert!(!ingredient.test_in(&context, &stack));
        stack.data_mut().set_other("minecraft:tooltip_style", None);
        stack.data_mut().set_other(
            "!minecraft:tooltip_style",
            Some(Tag::Compound(Default::default())),
        );
        assert!(!ingredient.test_in(&context, &stack));
    }
}

#[test]
fn rollup_boundary_and_absence_4109() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for inherited in [false, true] {
        let mut items = Items::empty();
        let defaults = crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:tooltip_style":"style"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        if inherited {
            items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
                "minecraft:stone".into(),
                defaults.clone(),
            )]));
        }
        let positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tooltip_style":"style"}}"#,
            &items,
        );
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:tooltip_style":{}}}"#,
            &items,
        );
        let mut stack = ItemStack::new("minecraft:stone", 1);
        assert_eq!(positive.test_in(&context, &stack), inherited);
        assert_eq!(absent.test_in(&context, &stack), !inherited);
        (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(defaults)));
        assert!(positive.test_in(&context, &stack));
        assert!(!absent.test_in(&context, &stack));
        let removal = Tag::compound([(
            "!minecraft:tooltip_style",
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
            "minecraft:tooltip_style",
            &tag,
            &registry,
        )
        .unwrap_err();
        assert!(!error.is_empty());
        let rule = IngredientRule::Components {
            patch: [("minecraft:tooltip_style".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:tooltip_style"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tooltip_style":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:tooltip_style"), "{error}");
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        let root = tempfile::tempdir().unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid_tooltip_style".into(), recipe(ingredient)),
                (
                    "fixture:valid".into(),
                    recipe(serde_json::json!("minecraft:stone")),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:invalid_tooltip_style",
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid",
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stone", 1))], 1)
                .is_some()
        );
    }
}
