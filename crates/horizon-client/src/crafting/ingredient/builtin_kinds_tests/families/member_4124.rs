use super::*;

#[test]
fn villager_food_codec_compiles_recipes_and_matches_borrowed_nutrition() {
    use horizon_command::item::decoded_component_value;
    use std::sync::Arc;

    let mut items = Items::empty();
    items.component_defaults = Arc::new(std::collections::HashMap::from([(
        "minecraft:carrot".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:villager_food":{"nutrition":3,"ignored":true}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let canonical = Tag::compound([("nutrition", Tag::Int(3))]);
    for nutrition in [
        Tag::Byte(3),
        Tag::Short(3),
        Tag::Int(3),
        Tag::Long(4_294_967_299),
        Tag::Float(3.9_f32.to_bits()),
        Tag::Double(3.9_f64.to_bits()),
    ] {
        assert_eq!(
            decoded_component_value(
                "minecraft:villager_food",
                &Tag::compound([("nutrition", nutrition), ("ignored", Tag::Byte(1))]),
                &items.component_registry,
            )
            .unwrap(),
            canonical,
        );
    }
    for nutrition in [
        Tag::Int(i32::MAX),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Double(f64::INFINITY.to_bits()),
    ] {
        assert_eq!(
            decoded_component_value(
                "minecraft:villager_food",
                &Tag::compound([("nutrition", nutrition)]),
                &items.component_registry,
            )
            .unwrap(),
            Tag::compound([("nutrition", Tag::Int(i32::MAX))]),
        );
    }
    for nutrition in [
        Tag::Int(0),
        Tag::Int(-1),
        Tag::Long(2_147_483_648),
        Tag::Long(4_294_967_296),
        Tag::Double(0.9_f64.to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Double(f64::NEG_INFINITY.to_bits()),
        Tag::String("3".into()),
    ] {
        assert!(
            decoded_component_value(
                "minecraft:villager_food",
                &Tag::compound([("nutrition", nutrition)]),
                &items.component_registry,
            )
            .is_err()
        );
    }

    let ingredient = |payload: Value, strict| {
        serde_json::json!({
            "type":"neoforge:components","items":"minecraft:carrot",
            "components":{"minecraft:villager_food":payload},"strict":strict
        })
    };
    let recipe = |value| {
        serde_json::json!({
            "type":"minecraft:crafting_shapeless","ingredients":[value],
            "result":{"id":"minecraft:diamond"}
        })
    };
    let mut sources = vec![
        (
            "fixture:villager_food".into(),
            recipe(ingredient(
                serde_json::json!({"nutrition":3.9,"unused":17}),
                true,
            )),
        ),
        (
            "fixture:wrapped".into(),
            recipe(ingredient(
                serde_json::json!({"nutrition":4_294_967_299_i64}),
                false,
            )),
        ),
    ];
    for (index, payload) in [
        serde_json::json!({}),
        serde_json::json!(3),
        serde_json::json!({"nutrition":0}),
        serde_json::json!({"nutrition":-1}),
        serde_json::json!({"nutrition":2_147_483_648_i64}),
        serde_json::json!({"nutrition":"3"}),
    ]
    .into_iter()
    .enumerate()
    {
        let value = ingredient(payload, false);
        let walked = WalkedIngredient::walk(
            &value,
            &items.item_tag_order,
            &|_| None,
            &resolved_items,
            &|data| crate::json_nbt::json_to_compound(data).ok(),
        )
        .unwrap();
        let error = components::validate(walked.rule.as_ref().unwrap(), &items).unwrap_err();
        assert!(error.contains("minecraft:villager_food"), "{error}");
        sources.push((format!("fixture:invalid_{index}"), recipe(value)));
    }
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let recipes = crate::crafting::Recipes::from_sources(
        &std::env::temp_dir(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 2);
    let mut carrot = ItemStack::new("minecraft:carrot", 1);
    let matches = |stack: &ItemStack| {
        recipes.recipe_matches(&context, "fixture:villager_food", &[Some(stack.clone())], 1)
    };
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:carrot","components":{"!minecraft:villager_food":{}}}"#,
        &items,
    );
    assert!(!absent.test_in(&context, &carrot));
    assert!(matches(&carrot), "prototype nutrition must participate");
    carrot.data_mut().set_loot_component(
        "minecraft:villager_food",
        &Some(Tag::compound([
            ("nutrition", Tag::Long(4_294_967_299)),
            ("different_unknown_field", Tag::String("ignored".into())),
        ])),
    );
    assert!(matches(&carrot));
    assert!(recipes.recipe_matches(&context, "fixture:wrapped", &[Some(carrot.clone())], 1,));
    let predicate = Ingredient::parse(
        &ingredient(serde_json::json!({"nutrition":3}), true),
        &items,
    )
    .unwrap();
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(predicate.test_in(&context, &carrot));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    carrot
        .data_mut()
        .set_loot_component("minecraft:custom_data", &Some(Tag::Int(1)));
    assert!(
        !matches(&carrot),
        "strict matching must reject extra components"
    );
    carrot = ItemStack::new("minecraft:carrot", 1);
    carrot.data_mut().set_loot_component(
        "minecraft:villager_food",
        &Some(Tag::compound([("nutrition", Tag::Int(4))])),
    );
    assert!(!matches(&carrot));
    assert!(!absent.test_in(&context, &carrot));
    let removal = Tag::compound([(
        "!minecraft:villager_food",
        Tag::Compound(Default::default()),
    )]);
    (_, carrot.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &carrot));
    assert!(
        !matches(&carrot),
        "explicit removal overrides the prototype"
    );
    assert!(!matches(&ItemStack::new("minecraft:carrot", 0)));
}

#[test]
fn rollup_rejection_and_absence_matrix_4124() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:villager_food":{"nutrition": 3}}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:villager_food":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(absent.test_in(&context, &stack));
    let payload = crate::json_nbt::json_to_compound(
        serde_json::json!({"minecraft:villager_food":{"nutrition": 3}})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    (_, stack.extra) = crate::item_data::decode_components(Some(&Tag::Compound(payload)));
    assert!(!absent.test_in(&context, &stack));
    assert!(positive.test_in(&context, &stack));
    let removal = Tag::compound([(
        "!minecraft:villager_food",
        Tag::Compound(Default::default()),
    )]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(absent.test_in(&context, &stack));
    assert!(!positive.test_in(&context, &stack));
    for tag in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:villager_food".into(), tag)]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:villager_food"), "{error}");
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(false),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:villager_food":value}});
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:villager_food"), "{error}");
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
