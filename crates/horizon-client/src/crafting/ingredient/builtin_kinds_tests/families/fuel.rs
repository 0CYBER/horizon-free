use super::*;

#[test]
fn cooking_fuel_zero_speed_is_valid_without_defaults_or_bounds() {
    let items = Items::empty();
    let value = Tag::compound([
        ("burn_time", Tag::Int(-5)),
        ("speed_multiplier", Tag::Int(0)),
        ("ignored", Tag::String("anything".into())),
    ]);
    assert_eq!(
        horizon_command::item::decoded_component_value(
            "minecraft:cooking_fuel",
            &value,
            &items.component_registry
        )
        .unwrap(),
        Tag::compound([
            ("burn_time", Tag::Int(-5)),
            ("speed_multiplier", Tag::Float(0))
        ])
    );
}

#[test]
fn cooking_fuel_recipe_compiles_and_matches_decoded_live_and_prototype_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:coal".into(),
        [(
            "minecraft:cooking_fuel",
            Tag::compound([
                ("burn_time", Tag::Long(4_294_967_306)),
                ("speed_multiplier", Tag::Short(2)),
                ("ignored", Tag::Int(100)),
            ]),
        )]
        .into_iter()
        .map(|(id, value)| (id.into(), value))
        .collect(),
    )]));
    let recipe = |payload: serde_json::Value| {
        serde_json::json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{
                "type":"neoforge:components",
                "items":"minecraft:coal",
                "components":{"minecraft:cooking_fuel":payload}
            }],
            "result":{"id":"minecraft:diamond"}
        })
    };
    let mut sources = vec![
        (
            "fixture:fuel_constant".into(),
            recipe(serde_json::json!({"burn_time":10.9,"speed_multiplier":2,"ignored":false})),
        ),
        (
            "fixture:fuel_reference".into(),
            recipe(serde_json::json!({"burn_time":"fuel/test","speed_multiplier":":speed"})),
        ),
    ];
    for (index, payload) in [
        serde_json::json!(10),
        serde_json::json!({"burn_time":10}),
        serde_json::json!({"speed_multiplier":2}),
        serde_json::json!({"burn_time":{},"speed_multiplier":2}),
        serde_json::json!({"burn_time":"Bad:key","speed_multiplier":2}),
        serde_json::json!({"burn_time":10,"speed_multiplier":[]}),
    ]
    .into_iter()
    .enumerate()
    {
        sources.push((format!("fixture:bad_fuel_{index}"), recipe(payload)));
    }
    let root = tempfile::tempdir().unwrap();
    let load = load();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    let expected = recipes.placement("fixture:fuel_constant").unwrap();
    assert_eq!(
        expected.ingredients[0].shown.as_ref().unwrap()[0]
            .data()
            .and_then(|data| data.other("minecraft:cooking_fuel")),
        Some(&Tag::compound([
            ("burn_time", Tag::Int(10)),
            ("speed_multiplier", Tag::Float(0x4000_0000)),
        ]))
    );
    assert_eq!(recipes.crafting.len(), 2);
    let ctx = load.loop_context(LoopSide::Server, []);
    let mut coal = ItemStack::new("minecraft:coal", 3);
    let matches = |stack: &ItemStack, id| {
        recipes.recipe_matches(&ctx, id, &[Some(stack.clone()), None, None, None], 2)
    };
    assert!(matches(&coal, "fixture:fuel_constant"));
    assert!(!matches(&coal, "fixture:fuel_reference"));
    coal.data_mut().set_other(
        "minecraft:cooking_fuel",
        Some(Tag::compound([
            ("burn_time", Tag::Double(10.1_f64.to_bits())),
            ("speed_multiplier", Tag::Double(2.0_f64.to_bits())),
        ])),
    );
    assert!(matches(&coal, "fixture:fuel_constant"));
    coal.data_mut().set_other(
        "minecraft:cooking_fuel",
        Some(Tag::compound([
            ("burn_time", Tag::String("minecraft:fuel/test".into())),
            ("speed_multiplier", Tag::String("speed".into())),
            ("ignored", Tag::List(vec![])),
        ])),
    );
    assert!(!matches(&coal, "fixture:fuel_constant"));
    assert!(matches(&coal, "fixture:fuel_reference"));
    let placement = recipes.placement("fixture:fuel_reference").unwrap();
    let ingredient = &placement.ingredients[0];
    // Warm the NBT lookup's thread-local key buffer before counting allocations.
    assert!(ingredient.test_in(&ctx, &coal));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&ctx, std::hint::black_box(&coal)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    for malformed in [
        Tag::End,
        Tag::Byte(0),
        Tag::List(vec![]),
        Tag::compound([("burn_time", Tag::Int(10))]),
    ] {
        coal.data_mut()
            .set_other("minecraft:cooking_fuel", Some(malformed));
        assert!(!matches(&coal, "fixture:fuel_constant"));
        assert!(!matches(&coal, "fixture:fuel_reference"));
    }
    coal.data_mut().set_other("minecraft:cooking_fuel", None);
    coal.data_mut().set_other(
        "!minecraft:cooking_fuel",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!matches(&coal, "fixture:fuel_constant"));
    assert!(!matches(&coal, "fixture:fuel_reference"));
    let malformed = IngredientRule::Components {
        patch: [(
            "minecraft:cooking_fuel".into(),
            Tag::compound([("burn_time", Tag::Int(10))]),
        )]
        .into_iter()
        .collect(),
        removed: vec![],
        strict: false,
    };
    let error = super::components::validate(&malformed, &items).unwrap_err();
    assert!(
        error.contains("component minecraft:cooking_fuel:"),
        "{error}"
    );
    assert!(error.contains("speed_multiplier"), "{error}");
}

#[test]
fn brewing_fuel_codec_compiles_recipes_and_borrows_live_values() {
    use horizon_core::nbt::JavaString;
    use serde_json::json;
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:blaze_powder".into(),
        [(
            "minecraft:brewing_fuel".into(),
            Tag::compound([
                ("uses", Tag::Byte(-2)),
                ("speed_multiplier", Tag::Double(1.25f64.to_bits())),
                ("ignored", Tag::Int(7)),
            ]),
        )]
        .into_iter()
        .collect(),
    )]));
    let ingredient_json = |payload: Value| {
        json!({
            "type":"neoforge:components", "items":"minecraft:blaze_powder",
            "components":{"minecraft:brewing_fuel":payload}
        })
    };
    let numeric = ingredient_json(json!({"uses":-2.75,"speed_multiplier":1.25,"ignored":false}));
    let reference =
        ingredient_json(json!({"uses":"missing/provider","speed_multiplier":":missing/speed"}));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipe = |ingredient: Value| {
        json!({
            "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
            "result":{"id":"minecraft:diamond","count":2}
        })
    };
    let mut sources = vec![
        ("fixture:numeric".into(), recipe(numeric.clone())),
        ("fixture:reference".into(), recipe(reference.clone())),
    ];
    for (n, payload) in [
        json!({}),
        json!({"uses":1}),
        json!({"speed_multiplier":1}),
        json!({"uses":{},"speed_multiplier":1}),
        json!({"uses":1,"speed_multiplier":[]}),
        json!({"uses":"UPPER:key","speed_multiplier":1}),
        json!({"uses":1,"speed_multiplier":"..:key"}),
    ]
    .into_iter()
    .enumerate()
    {
        let invalid = ingredient_json(payload);
        assert!(Ingredient::parse(&invalid, &items).is_none());
        sources.push((format!("fixture:invalid_{n}"), recipe(invalid)));
    }
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 2);
    let mut stack = ItemStack::new("minecraft:blaze_powder", 1);
    let (id, result) = recipes.find(&context, &[Some(stack.clone())], 1).unwrap();
    assert_eq!(id, "fixture:numeric");
    assert_eq!(result.item.as_ref(), "minecraft:diamond");
    assert_eq!(result.count, 2);
    let ingredient = Ingredient::parse(&numeric, &items).unwrap();
    assert!(ingredient.test_in(&context, &stack));
    stack.data_mut().set_other(
        "minecraft:brewing_fuel",
        Some(Tag::compound([
            ("uses", Tag::Long(4_294_967_294)),
            ("speed_multiplier", Tag::Float(1.25f32.to_bits())),
            ("ignored", Tag::String("different".into())),
        ])),
    );
    assert_eq!(
        recipes.find(&context, &[Some(stack.clone())], 1).unwrap().0,
        "fixture:numeric"
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(
                ingredient.test_in(&context, std::hint::black_box(&stack))
            ));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stack.data_mut().set_other(
        "minecraft:brewing_fuel",
        Some(Tag::compound([
            ("uses", Tag::String("minecraft:missing/provider".into())),
            ("speed_multiplier", Tag::String("missing/speed".into())),
        ])),
    );
    assert_eq!(
        recipes.find(&context, &[Some(stack.clone())], 1).unwrap().0,
        "fixture:reference"
    );
    assert!(
        !ingredient.test_in(&context, &stack),
        "a reference is not a numeric constant"
    );
    let reference = Ingredient::parse(&reference, &items).unwrap();
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(reference.test_in(&context, &stack));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for malformed in [
        Tag::compound([("uses", Tag::Int(-2))]),
        Tag::compound([
            ("uses", Tag::Int(-2)),
            ("speed_multiplier", Tag::String(JavaString(vec![0xd800]))),
        ]),
        Tag::compound([
            ("uses", Tag::Int(-2)),
            ("speed_multiplier", Tag::String("UPPER".into())),
        ]),
    ] {
        stack
            .data_mut()
            .set_other("minecraft:brewing_fuel", Some(malformed));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.find(&context, &[Some(stack.clone())], 1).is_none());
    }
    let decode = |uses, speed| {
        horizon_command::item::decoded_component_value(
            "minecraft:brewing_fuel",
            &Tag::compound([("uses", uses), ("speed_multiplier", speed)]),
            &items.component_registry,
        )
        .unwrap()
    };
    for (input, expected) in [
        (Tag::Byte(-2), -2),
        (Tag::Short(-2), -2),
        (Tag::Int(-2), -2),
        (Tag::Long(4_294_967_294), -2),
        (Tag::Float((-2.75f32).to_bits()), -2),
        (Tag::Double(f64::NAN.to_bits()), 0),
        (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
        (Tag::Double(f64::NEG_INFINITY.to_bits()), i32::MIN),
    ] {
        assert_eq!(
            decode(input, Tag::Double(1.25f64.to_bits())),
            Tag::compound([
                ("uses", Tag::Int(expected)),
                ("speed_multiplier", Tag::Float(1.25f32.to_bits())),
            ])
        );
    }
    for (input, expected) in [
        (Tag::Byte(-1), (-1.0f32).to_bits()),
        (Tag::Short(2), 2f32.to_bits()),
        (Tag::Int(16_777_217), 16_777_216f32.to_bits()),
        (Tag::Long(i64::MAX), (i64::MAX as f32).to_bits()),
        (
            Tag::Double(f64::INFINITY.to_bits()),
            f32::INFINITY.to_bits(),
        ),
    ] {
        assert_eq!(
            decode(Tag::Int(1), input),
            Tag::compound([
                ("uses", Tag::Int(1)),
                ("speed_multiplier", Tag::Float(expected)),
            ])
        );
    }
    let nan = decode(Tag::Int(1), Tag::Float(0x7f80_0001));
    // NaN payload bits are not part of boxed-float equality.
    assert!(matches!(
        nan.get("speed_multiplier"),
        Some(Tag::Float(value)) if f32::from_bits(*value).is_nan()
    ));
    for (expected, actual, matches) in [
        (0f32.to_bits(), (-0f32).to_bits(), false),
        (f32::NAN.to_bits(), 0xff80_0001, true),
        (f32::INFINITY.to_bits(), f32::INFINITY.to_bits(), true),
    ] {
        let raw = Tag::compound([
            ("uses", Tag::Int(1)),
            ("speed_multiplier", Tag::Float(expected)),
        ]);
        let mut a = ItemStack::new("minecraft:blaze_powder", 1);
        a.data_mut().set_other("minecraft:brewing_fuel", Some(raw));
        stack.data_mut().set_other(
            "minecraft:brewing_fuel",
            Some(Tag::compound([
                ("uses", Tag::Int(1)),
                ("speed_multiplier", Tag::Float(actual)),
            ])),
        );
        assert_eq!(
            horizon_items::item_data::ingredient_components::component_equal(
                "minecraft:brewing_fuel",
                &a,
                &stack,
            ),
            matches
        );
    }
}

#[test]
fn compostable_layers_compile_real_recipes_and_match_borrowed_values() {
    const COMPONENT: &str = "minecraft:compostable";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:apple".into(),
        std::collections::BTreeMap::from([(
            COMPONENT.into(),
            Tag::compound([("layers", Tag::Short(2)), ("ignored", Tag::Byte(1))]),
        )]),
    )]));
    let ingredient = |payload: Value| {
        serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:apple",
            "components":{COMPONENT:payload}
        })
    };
    let constant_json =
        ingredient(serde_json::json!({"layers":2,"chance":"ignored","can_villager_compost":false}));
    let reference_json = ingredient(serde_json::json!({"layers":"fixture:unregistered"}));
    let constant = Ingredient::parse(&constant_json, &items).unwrap();
    let reference = Ingredient::parse(&reference_json, &items).unwrap();
    let mut strict_json = constant_json.clone();
    strict_json["strict"] = Value::Bool(true);
    let strict = Ingredient::parse(&strict_json, &items).unwrap();
    assert_eq!(
        constant.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other(COMPONENT),
        Some(&Tag::compound([("layers", Tag::Int(2))])),
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipe = |value: Value| {
        serde_json::json!({
            "type":"minecraft:crafting_shapeless", "ingredients":[value],
            "result":{"id":"minecraft:diamond"}
        })
    };
    let invalid_payloads = [
        serde_json::json!({}),
        serde_json::json!({"chance":0.5,"can_villager_compost":true}),
        serde_json::json!({"layers":[]}),
        serde_json::json!({"layers":{}}),
        serde_json::json!({"layers":"Bad:provider"}),
        serde_json::json!({"layers":"minecraft:a:b"}),
        serde_json::json!(2),
    ];
    let mut sources = vec![
        ("fixture:constant".into(), recipe(constant_json)),
        ("fixture:reference".into(), recipe(reference_json)),
    ];
    for (index, payload) in invalid_payloads.into_iter().enumerate() {
        let json = ingredient(payload);
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        assert!(
            components::validate(&rule, &items)
                .unwrap_err()
                .starts_with("component minecraft:compostable:")
        );
        sources.push((format!("fixture:invalid_{index}"), recipe(json)));
    }
    assert!(
        horizon_command::item::decoded_component_value(
            COMPONENT,
            &Tag::compound([("layers", Tag::End)]),
            &items.component_registry,
        )
        .is_err()
    );
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 2, "malformed recipes are omitted");
    let mut candidate = ItemStack::new("minecraft:apple", 1);
    assert!(recipes.recipe_matches(&context, "fixture:constant", &[Some(candidate.clone())], 1));
    assert!(!recipes.recipe_matches(&context, "fixture:reference", &[Some(candidate.clone())], 1));
    assert!(strict.test_in(&context, &candidate));
    candidate.data_mut().set_loot_component(
        "minecraft:custom_data",
        &Some(Tag::compound([("opaque", Tag::Int(1))])),
    );
    assert!(constant.test_in(&context, &candidate));
    assert!(!strict.test_in(&context, &candidate));
    candidate
        .data_mut()
        .set_loot_component("minecraft:custom_data", &None);
    for (number, expected) in [
        (Tag::Byte(2), 2),
        (Tag::Short(-2), -2),
        (Tag::Int(i32::MIN), i32::MIN),
        (Tag::Long(4_294_967_298), 2),
        (Tag::Float(2.9_f32.to_bits()), 2),
        (Tag::Double((-2.9_f64).to_bits()), -2),
        (Tag::Double(f64::NAN.to_bits()), 0),
        (Tag::Float(f32::INFINITY.to_bits()), i32::MAX),
        (Tag::Double(f64::NEG_INFINITY.to_bits()), i32::MIN),
        (Tag::Double((-0.0_f64).to_bits()), 0),
    ] {
        let payload = Tag::compound([("layers", number), ("unknown", Tag::List(vec![]))]);
        assert_eq!(
            horizon_command::item::decoded_component_value(
                COMPONENT,
                &payload,
                &items.component_registry
            )
            .unwrap(),
            Tag::compound([("layers", Tag::Int(expected))]),
        );
        candidate
            .data_mut()
            .set_loot_component(COMPONENT, &Some(payload));
        assert_eq!(constant.test_in(&context, &candidate), expected == 2);
        assert_eq!(
            recipes.recipe_matches(&context, "fixture:constant", &[Some(candidate.clone())], 1),
            expected == 2
        );
    }
    candidate.data_mut().set_loot_component(
        COMPONENT,
        &Some(Tag::compound([(
            "layers",
            Tag::String("fixture:unregistered".into()),
        )])),
    );
    assert!(recipes.recipe_matches(&context, "fixture:reference", &[Some(candidate.clone())], 1));
    assert!(!constant.test_in(&context, &candidate));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(reference.test_in(&context, std::hint::black_box(&candidate)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    let default_key = Ingredient::parse(
        &ingredient(serde_json::json!({"layers":"composter"})),
        &items,
    )
    .unwrap();
    assert_eq!(
        default_key.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other(COMPONENT),
        Some(&Tag::compound([(
            "layers",
            Tag::String("minecraft:composter".into())
        )])),
    );
    for spelling in ["composter", ":composter", "minecraft:composter"] {
        candidate.data_mut().set_loot_component(
            COMPONENT,
            &Some(Tag::compound([("layers", Tag::String(spelling.into()))])),
        );
        assert!(default_key.test_in(&context, &candidate));
    }
    for spelling in ["", ":", "minecraft:"] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                COMPONENT,
                &Tag::compound([("layers", Tag::String(spelling.into()))]),
                &items.component_registry,
            )
            .unwrap(),
            Tag::compound([("layers", Tag::String("minecraft:".into()))]),
        );
    }
    let malformed = Tag::compound([(
        "layers",
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
    )]);
    assert!(
        horizon_command::item::decoded_component_value(
            COMPONENT,
            &malformed,
            &items.component_registry
        )
        .is_err()
    );
    candidate
        .data_mut()
        .set_loot_component(COMPONENT, &Some(malformed));
    assert!(!reference.test_in(&context, &candidate));
    candidate.data_mut().set_loot_component(COMPONENT, &None);
    assert!(
        constant.test_in(&context, &candidate),
        "prototype is inherited again"
    );
    candidate.data_mut().set_loot_component(
        "!minecraft:compostable",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(
        !constant.test_in(&context, &candidate),
        "removal is not an inherited value"
    );
}

#[test]
fn food_codec_compiles_recipes_and_matches_borrowed_values() {
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    const COMPONENT: &str = "minecraft:food";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:apple".into(),
        std::collections::BTreeMap::from([(
            COMPONENT.into(),
            Tag::compound([
                ("nutrition", Tag::Long(4_294_967_300)),
                ("saturation", Tag::Double((-2.5_f64).to_bits())),
                ("ignored", Tag::Int(99)),
            ]),
        )]),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let payload = json!({"nutrition":4.9,"saturation":-2.5,"effects":"ignored legacy field"});
    let ingredient = |food: Value| {
        json!({"type":"neoforge:components","items":"minecraft:apple",
            "components":{"minecraft:food":food}})
    };
    let recipe = |food: Value| {
        json!({"type":"minecraft:crafting_shaped","pattern":["A"],
            "key":{"A":ingredient(food)},"result":{"id":"minecraft:diamond"}})
    };
    let mut sources = vec![("fixture:food".into(), recipe(payload.clone()))];
    for (index, invalid) in [
        json!(true),
        json!(false),
        json!(null),
        json!("food"),
        json!({"nutrition":"4","saturation":0}),
        json!({"nutrition":4,"saturation":0,"can_always_eat":{}}),
        json!({"nutrition":4,"saturation":0,"can_always_eat":[]}),
        json!({}),
        json!({"nutrition":4}),
        json!({"saturation":0}),
        json!({"nutrition":-1,"saturation":0}),
        json!({"nutrition":4,"saturation":"0"}),
        json!({"nutrition":4,"saturation":0,"can_always_eat":"false"}),
        json!([]),
    ]
    .into_iter()
    .enumerate()
    {
        let value = ingredient(invalid.clone());
        let error = components::validate_recipe_components(&value, &items).unwrap_err();
        assert!(error.contains("minecraft:food"), "{error}");
        let reload = crate::crafting::Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![
                ("fixture:food".into(), recipe(payload.clone())),
                ("fixture:invalid".into(), recipe(invalid.clone())),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(reload.crafting.len(), 1);
        let grid = [Some(ItemStack::new("minecraft:apple", 1))];
        assert!(reload.recipe_matches(&context, "fixture:food", &grid, 1));
        assert!(reload.craft(&context, &grid, 1).is_some());
        sources.push((format!("fixture:invalid_food_{index}"), recipe(invalid)));
    }
    for value in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([(COMPONENT.into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:food"), "{error}");
    }
    let recipes = crate::crafting::Recipes::from_sources(
        &std::env::temp_dir(),
        &items,
        &PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert_eq!(recipes.crafting[0].0, "fixture:food");
    let parsed = Ingredient::parse(&ingredient(payload.clone()), &items).unwrap();
    assert_eq!(
        parsed.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other(COMPONENT),
        Some(&Tag::compound([
            ("nutrition", Tag::Int(4)),
            ("saturation", Tag::Float((-2.5_f32).to_bits())),
            ("can_always_eat", Tag::Byte(0)),
        ])),
    );
    let mut grid = [Some(ItemStack::new("minecraft:apple", 1))];
    assert!(recipes.recipe_matches(&context, "fixture:food", &grid, 1));
    for (nutrition, saturation, always, expected) in [
        (Tag::Float(4.9_f32.to_bits()), Tag::Short(-2), None, false),
        (
            Tag::Double(4.9_f64.to_bits()),
            Tag::Double((-2.5_f64).to_bits()),
            None,
            true,
        ),
        (
            Tag::Byte(4),
            Tag::Float((-2.5_f32).to_bits()),
            Some(Tag::Int(256)),
            false,
        ),
        (
            Tag::Short(4),
            Tag::Float((-2.5_f32).to_bits()),
            Some(Tag::Double((-0.0_f64).to_bits())),
            true,
        ),
        (
            Tag::Long(4_294_967_300),
            Tag::Float((-2.5_f32).to_bits()),
            Some(Tag::Byte(0)),
            true,
        ),
    ] {
        let mut fields = vec![("nutrition", nutrition), ("saturation", saturation)];
        if let Some(always) = always {
            fields.push(("can_always_eat", always));
        }
        grid[0]
            .as_mut()
            .unwrap()
            .data_mut()
            .set_other(COMPONENT, Some(Tag::compound(fields)));
        assert_eq!(
            recipes.recipe_matches(&context, "fixture:food", &grid, 1),
            expected
        );
    }
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(parsed.test_in(&context, std::hint::black_box(grid[0].as_ref().unwrap())));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    grid[0]
        .as_mut()
        .unwrap()
        .data_mut()
        .set_loot_component(COMPONENT, &None);
    grid[0]
        .as_mut()
        .unwrap()
        .data_mut()
        .set_loot_component("!minecraft:food", &Some(Tag::Compound(Default::default())));
    assert!(!recipes.recipe_matches(&context, "fixture:food", &grid, 1));

    let absent = parse(
        r#"{"type":"neoforge:components","items":["minecraft:apple","minecraft:paper"],"components":{"!minecraft:food":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, grid[0].as_ref().unwrap()));
    assert!(!absent.test_in(&context, &ItemStack::new("minecraft:apple", 1)));
    let source = json!({"type":"neoforge:components","items":"minecraft:paper",
        "components":{"minecraft:food":payload},"strict":true});
    let strict = Ingredient::parse(&source, &items).unwrap();
    let mut relaxed_source = source.clone();
    relaxed_source["strict"] = json!(false);
    let relaxed = Ingredient::parse(&relaxed_source, &items).unwrap();
    let mut paper = ItemStack::new("minecraft:paper", 1);
    assert!(!strict.test_in(&context, &paper));
    assert!(absent.test_in(&context, &paper));
    paper.data_mut().set_other(
        COMPONENT,
        Some(Tag::compound([
            ("nutrition", Tag::Int(4)),
            ("saturation", Tag::Float((-2.5_f32).to_bits())),
            ("ignored", Tag::String("not an extra component".into())),
        ])),
    );
    assert!(strict.test_in(&context, &paper));
    assert!(!absent.test_in(&context, &paper));
    paper
        .data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(7)));
    assert!(!strict.test_in(&context, &paper));
    assert!(relaxed.test_in(&context, &paper));

    // The direct float codec allows nonfinite values; record equality uses
    // Float.compare (canonical NaNs and distinct signed zero), not Rust ==.
    for (a, b, equal) in [
        (f32::NAN.to_bits(), 0x7f800001, true),
        (0.0_f32.to_bits(), (-0.0_f32).to_bits(), false),
        (f32::INFINITY.to_bits(), f32::INFINITY.to_bits(), true),
    ] {
        let stack = |bits| {
            let mut stack = ItemStack::new("minecraft:apple", 1);
            stack.data_mut().set_other(
                COMPONENT,
                Some(Tag::compound([
                    ("nutrition", Tag::Int(4)),
                    ("saturation", Tag::Float(bits)),
                ])),
            );
            stack
        };
        let a = stack(a);
        let b = stack(b);
        assert_eq!(
            horizon_items::item_data::ingredient_components::component_equal(COMPONENT, &a, &b),
            equal,
        );
        assert!(
            horizon_command::item::decoded_component_value(
                COMPONENT,
                a.data().unwrap().other(COMPONENT).unwrap(),
                &items.component_registry,
            )
            .is_ok()
        );
    }
    let numeric = Tag::compound([
        ("nutrition", Tag::Double(f64::NAN.to_bits())),
        ("saturation", Tag::Long(16_777_217)),
        ("can_always_eat", Tag::Double(f64::NAN.to_bits())),
        ("obsolete", Tag::String("discarded".into())),
    ]);
    assert_eq!(
        horizon_command::item::decoded_component_value(
            COMPONENT,
            &numeric,
            &items.component_registry,
        )
        .unwrap(),
        Tag::compound([
            ("nutrition", Tag::Int(0)),
            ("saturation", Tag::Float(16_777_216.0_f32.to_bits())),
            ("can_always_eat", Tag::Byte(1)),
        ]),
    );
}
