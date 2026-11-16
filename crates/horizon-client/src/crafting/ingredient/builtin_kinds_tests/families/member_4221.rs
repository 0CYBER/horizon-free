use super::*;

fn tropical_fish_pattern_recipes(
    items: &Items,
    patch: serde_json::Value,
    strict: bool,
) -> crate::crafting::Recipes {
    let ingredient = serde_json::json!({
        "type": "neoforge:components",
        "items": "minecraft:tropical_fish_bucket",
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
                "fixture:tropical_fish_shapeless".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient.clone()],
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:tropical_fish_shaped".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["A"],
                    "key": {"A": ingredient.clone()},
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:tropical_fish_smelting".into(),
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
fn tropical_fish_pattern_recipe_codec_matches_exact_names_without_candidate_allocations() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    // TropicalFish.Pattern serialized names; buckets have no prototype pattern.
    for name in [
        "kob",
        "sunstreak",
        "snooper",
        "dasher",
        "brinely",
        "spotty",
        "flopper",
        "stripey",
        "glitter",
        "blockfish",
        "betty",
        "clayfish",
    ] {
        let different = if name == "kob" { "clayfish" } else { "kob" };
        // An unqualified component key normalizes, but the enum payload does not.
        let recipes = tropical_fish_pattern_recipes(
            &items,
            serde_json::json!({"tropical_fish/pattern": name}),
            false,
        );
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
                    .other("minecraft:tropical_fish/pattern"),
                Some(&Tag::String(name.into()))
            );
            assert!(!ingredient.test_in(
                &context,
                &ItemStack::new("minecraft:tropical_fish_bucket", 1)
            ));
            let mut stack = ItemStack::new("minecraft:tropical_fish_bucket", 7);
            stack.data_mut().set_other(
                "minecraft:tropical_fish/pattern",
                Some(Tag::String(name.into())),
            );
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..100 {
                    assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
            for value in [
                Tag::String(different.into()),
                Tag::Int(if name == "kob" { 0 } else { 1 }),
                Tag::String(horizon_core::nbt::JavaString(vec![114, 101, 100, 0xd800])),
            ] {
                stack
                    .data_mut()
                    .set_other("minecraft:tropical_fish/pattern", Some(value));
                assert!(!ingredient.test_in(&context, &stack));
            }
            stack.data_mut().set_other(
                "minecraft:tropical_fish/pattern",
                Some(Tag::String(name.into())),
            );
            stack.count = 0;
            assert!(!ingredient.test_in(&context, &stack));
        }
        let mut egg = ItemStack::new("minecraft:tropical_fish_bucket", 1);
        egg.data_mut().set_other(
            "minecraft:tropical_fish/pattern",
            Some(Tag::String(name.into())),
        );
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
            "minecraft:tropical_fish/pattern",
            Some(Tag::String(different.into())),
        );
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_none());
    }
}

#[test]
fn tropical_fish_pattern_recipe_defaults_removal_and_strictness_preserve_presence() {
    // Synthetic modded prototype, not a vanilla bucket or mob spawn default.
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:tropical_fish_bucket".into(),
        match Tag::compound([("minecraft:tropical_fish/pattern", Tag::String("kob".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let exact = tropical_fish_pattern_recipes(
        &items,
        serde_json::json!({"minecraft:tropical_fish/pattern": "kob"}),
        true,
    );
    let loose = tropical_fish_pattern_recipes(
        &items,
        serde_json::json!({"minecraft:tropical_fish/pattern": "kob"}),
        false,
    );
    let omitted = tropical_fish_pattern_recipes(&items, serde_json::json!({}), true);
    // Removal payloads require registration, not a successful variant decode.
    let removal = tropical_fish_pattern_recipes(
        &items,
        serde_json::json!({"!minecraft:tropical_fish/pattern": 123}),
        true,
    );
    assert_eq!(removal.crafting.len(), 2);
    let mut egg = ItemStack::new("minecraft:tropical_fish_bucket", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    let removed = Tag::compound([(
        "!minecraft:tropical_fish/pattern",
        Tag::compound([] as [(&str, Tag); 0]),
    )]);
    (_, egg.extra) = crate::item_data::decode_components(Some(&removed));
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:tropical_fish/pattern",
        &Some(Tag::String("clayfish".into())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
    let absent = tropical_fish_pattern_recipes(
        &Items::empty(),
        serde_json::json!({"!minecraft:tropical_fish/pattern": {}}),
        true,
    );
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:tropical_fish_bucket", 1))],
                1
            )
            .is_some()
    );
}

#[test]
fn tropical_fish_pattern_invalid_codec_payloads_omit_whole_recipes() {
    let items = Items::empty();
    for (payload, diagnostic) in [
        (serde_json::json!(0), "Not a string"),
        (serde_json::json!(1), "Not a string"),
        (serde_json::json!(1281), "Not a string"),
        (serde_json::json!(1.0), "Not a string"),
        (serde_json::json!(false), "Not a string"),
        (serde_json::json!({}), "Not a string"),
        (serde_json::json!([]), "Not a string"),
        (serde_json::json!("KOB"), "Unknown element name:KOB"),
        (serde_json::json!("arctic"), "Unknown element name:arctic"),
        (
            serde_json::json!("minecraft:kob"),
            "Unknown element name:minecraft:kob",
        ),
        (serde_json::json!(""), "Unknown element name:"),
    ] {
        let patch = serde_json::json!({"minecraft:tropical_fish/pattern": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        assert_eq!(
            components::validate(&rule, &items),
            Err(format!(
                "component minecraft:tropical_fish/pattern: {diagnostic}"
            ))
        );
        let ingredient = serde_json::json!({
            "type": "neoforge:components",
            "items": "minecraft:tropical_fish_bucket",
            "components": patch.clone()
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = tropical_fish_pattern_recipes(&items, patch, false);
        assert!(recipes.crafting.is_empty());
        assert!(recipes.cooking.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn rollup_rejection_and_absence_matrix_4221() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tropical_fish/pattern":"kob"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:tropical_fish/pattern":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:tropical_fish/pattern":"kob"})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([(
        "!minecraft:tropical_fish/pattern",
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
            .set_loot_component("minecraft:tropical_fish/pattern", &Some(malformed.clone()));
        assert!(!positive.test_in(&context, &stack));
        let rule = IngredientRule::Components {
            patch: [("minecraft:tropical_fish/pattern".into(), malformed)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        assert!(
            components::validate(&rule, &items)
                .unwrap_err()
                .contains("minecraft:tropical_fish/pattern")
        );
    }
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:tropical_fish/pattern".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:tropical_fish/pattern"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(1281),
        serde_json::json!(1.0),
        serde_json::json!({}),
        serde_json::json!("KOB"),
        serde_json::json!("arctic"),
        serde_json::json!("minecraft:kob"),
        serde_json::json!(""),
        serde_json::json!(false),
        serde_json::json!([]),
        serde_json::json!("unknown"),
        serde_json::json!("Minecraft:kob"),
        serde_json::json!("kob".to_uppercase()),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tropical_fish/pattern":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:tropical_fish/pattern"), "{error}");
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
fn inherited_setter_removal_4221() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for inherited in [false, true] {
        let mut items = Items::empty();
        if inherited {
            let Tag::Compound(defaults) =
                Tag::compound([("minecraft:tropical_fish/pattern", Tag::String("kob".into()))])
            else {
                unreachable!()
            };
            items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
                "minecraft:stone".into(),
                defaults,
            )]));
        }
        let positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tropical_fish/pattern":"kob"}}"#,
            &items,
        );
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:tropical_fish/pattern":{}}}"#,
            &items,
        );
        let mut stack = ItemStack::new("minecraft:stone", 1);
        assert_eq!(positive.test_in(&context, &stack), inherited);
        assert_eq!(absent.test_in(&context, &stack), !inherited);
        stack.data_mut().set_loot_component(
            "minecraft:tropical_fish/pattern",
            &Some(Tag::String("kob".into())),
        );
        assert!(positive.test_in(&context, &stack));
        stack
            .data_mut()
            .set_loot_component("minecraft:tropical_fish/pattern", &None);
        stack.data_mut().set_loot_component(
            "!minecraft:tropical_fish/pattern",
            &Some(Tag::Compound(Default::default())),
        );
        assert!(absent.test_in(&context, &stack));
        assert!(!positive.test_in(&context, &stack));
    }
}
