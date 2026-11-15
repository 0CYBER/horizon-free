use super::*;

#[test]
#[ignore = "daily"]
fn components_match_damage_ignore_count_and_refuse_empty() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 3);
    sword.damage = 5;
    assert!(ingredient.test_in(&context, &sword));
    sword.damage = 0;
    assert!(!ingredient.test_in(&context, &sword));
    sword.damage = 5;
    sword.count = 0;
    assert!(!ingredient.test_in(&context, &sword));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:air", 1)));
    assert!(!ingredient.is_simple());
}

#[test]
fn components_map_id_decodes_signed_numbers_and_matches_live_values() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7.75}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":4294967289}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::validate_recipe(&json, &items, "fixture:map_id"));
        let ingredient = Ingredient::parse(&json, &items).unwrap();
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0].data().unwrap().map_id,
            Some(-7),
        );
        for value in [
            Tag::Byte(-7),
            Tag::Short(-7),
            Tag::Int(-7),
            Tag::Long(4_294_967_289),
            Tag::Float((-7.75_f32).to_bits()),
            Tag::Double((-7.75_f64).to_bits()),
        ] {
            assert_eq!(
                horizon_command::item::decoded_component_value(
                    "minecraft:map_id",
                    &value,
                    &items.component_registry,
                )
                .unwrap(),
                Tag::Int(-7),
            );
            let mut stack = ItemStack::new("minecraft:filled_map", 1);
            stack
                .data_mut()
                .set_other("minecraft:map_id", Some(value.clone()));
            assert!(ingredient.test_in(&context, &stack));
            let patch = Tag::compound([("minecraft:map_id", value)]);
            (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
            assert_eq!(stack.data().unwrap().map_id, Some(-7));
            assert!(ingredient.test_in(&context, &stack));
        }
        let mut different = ItemStack::new("minecraft:filled_map", 1);
        different.data_mut().map_id = Some(-6);
        assert!(!ingredient.test_in(&context, &different));
        assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:filled_map", 1)));
    }

    let bad: Value = serde_json::from_str(
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":"-7"}}"#,
    )
    .unwrap();
    let rule = IngredientRule::Components {
        patch: crate::json_nbt::json_to_compound(bad["components"].as_object().unwrap()).unwrap(),
        removed: Vec::new(),
        strict: false,
    };
    assert_eq!(
        components::validate(&rule, &items).unwrap_err(),
        "component minecraft:map_id: Not a number",
    );
    assert!(Ingredient::parse(&bad, &items).is_none());
    let bad_recipe = serde_json::json!({
        "type": "minecraft:crafting_shapeless",
        "ingredients": [bad],
        "result": {"id": "minecraft:map"},
    });
    assert!(!Ingredient::validate_recipe(
        &bad_recipe,
        &items,
        "fixture:bad_map_id"
    ));

    let mut defaults = Items::empty();
    defaults.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:filled_map".into(),
        [(
            "minecraft:map_id".into(),
            Tag::Double((-7.75_f64).to_bits()),
        )]
        .into_iter()
        .collect(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7}}"#,
        &defaults,
    );
    let mut inherited = ItemStack::new("minecraft:filled_map", 1);
    assert!(ingredient.test_in(&context, &inherited));
    inherited.data_mut().map_id = Some(-6);
    assert!(!ingredient.test_in(&context, &inherited));

    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"!minecraft:map_id":{}}}"#;
    let absence = parse(absence_source, &defaults);
    let mut removed = ItemStack::new("minecraft:filled_map", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_other(
        "!minecraft:map_id",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!ingredient.test_in(&context, &removed));
    let no_default_absence = parse(absence_source, &items);
    let mut bare = ItemStack::new("minecraft:filled_map", 1);
    assert!(no_default_absence.test_in(&context, &bare));
    bare.data_mut().map_id = Some(-7);
    assert!(!no_default_absence.test_in(&context, &bare));
}

#[test]
#[ignore = "daily"]
fn components_strict_counts_max_damage_default_and_custom_name() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5},"strict":true}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.damage = 5;
    // MAX_DAMAGE is a real prototype component, absent from the literal patch.
    assert!(
        items.component_defaults["minecraft:diamond_sword"]
            .contains_key(&"minecraft:max_damage".into())
    );
    assert!(!ingredient.test_in(&context, &sword));
    sword.data_mut().set_loot_component(
        "minecraft:custom_name",
        &Some(Tag::String("Named sword".into())),
    );
    assert!(!ingredient.test_in(&context, &sword));

    // A literal supported-only prototype isolates strict extra-component behavior.
    let mut supported = Items::empty();
    supported.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([
        ("minecraft:diamond_sword".into(), crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:damage":0,"minecraft:max_damage":1561,"minecraft:max_stack_size":1}).as_object().unwrap()).unwrap())
    ]));
    let complete = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5,"minecraft:max_damage":1561,"minecraft:max_stack_size":1},"strict":true}"#,
        &supported,
    );
    let mut plain = ItemStack::new("minecraft:diamond_sword", 1);
    plain.damage = 5;
    assert!(complete.test_in(&context, &plain));
    assert!(!complete.test_in(&context, &sword));
    let missing_max = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5,"minecraft:max_stack_size":1},"strict":true}"#,
        &supported,
    );
    assert!(
        !missing_max.test_in(&context, &plain),
        "the literal max_damage default must be named"
    );
}

#[test]
fn ominous_bottle_amplifier_codec_loads_matches_and_rejects_out_of_range() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:ominous_bottle".into(),
        [("minecraft:ominous_bottle_amplifier".into(), Tag::Byte(0))]
            .into_iter()
            .collect(),
    )]));
    let source = r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":3.9}}"#;
    let ingredient = parse(source, &items);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .ominous_amplifier,
        Some(3)
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:ominous_amplifier".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[serde_json::from_str::<Value>(source).unwrap()],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
    bottle.data_mut().ominous_amplifier = Some(3);
    assert!(ingredient.test_in(&context, &bottle));
    assert_eq!(
        recipes
            .craft(&context, &[Some(bottle.clone())], 1)
            .unwrap()
            .item
            .as_ref(),
        "minecraft:diamond"
    );
    bottle.data_mut().ominous_amplifier = Some(4);
    assert!(!ingredient.test_in(&context, &bottle));
    assert!(
        recipes
            .craft(&context, &[Some(bottle.clone())], 1)
            .is_none()
    );

    // Codec.INT uses Number.intValue, including long truncation and fractional floats.
    for value in [
        Tag::Byte(3),
        Tag::Short(3),
        Tag::Int(3),
        Tag::Long(4_294_967_299),
        Tag::Float(3.9f32.to_bits()),
        Tag::Double(3.9f64.to_bits()),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:ominous_bottle_amplifier",
                &value,
                &items.component_registry,
            )
            .unwrap(),
            Tag::Int(3)
        );
        bottle
            .data_mut()
            .set_other("minecraft:ominous_bottle_amplifier", Some(value.clone()));
        let (matches, allocations) =
            horizon_threads::allocation_probe::measure(|| ingredient.test_in(&context, &bottle));
        assert!(matches);
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        let patch = Tag::compound([("minecraft:ominous_bottle_amplifier", value)]);
        (_, bottle.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(bottle.data().unwrap().ominous_amplifier, Some(3));
        assert!(ingredient.test_in(&context, &bottle));
    }
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(ingredient.test_in(&context, &bottle)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");

    let zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":0}}"#,
        &items,
    );
    let four = parse(
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4}}"#,
        &items,
    );
    let plain = ItemStack::new("minecraft:ominous_bottle", 1);
    let (matches, allocations) =
        horizon_threads::allocation_probe::measure(|| zero.test_in(&context, &plain));
    assert!(matches);
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    bottle.data_mut().ominous_amplifier = Some(4);
    assert!(four.test_in(&context, &bottle));
    let mut live_four = ItemStack::new("minecraft:ominous_bottle", 1);
    live_four.data_mut().ominous_amplifier = Some(4);
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4.0}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4.5}}"#,
    ] {
        let boundary = parse(source, &items);
        assert_eq!(
            boundary.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .ominous_amplifier,
            Some(4)
        );
        assert!(boundary.test_in(&context, &live_four));
    }
    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"!minecraft:ominous_bottle_amplifier":{}}}"#;
    let absence = parse(absence_source, &items);
    let mut removed = ItemStack::new("minecraft:ominous_bottle", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_other(
        "!minecraft:ominous_bottle_amplifier",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!zero.test_in(&context, &removed));
    assert!(!ingredient.test_in(&context, &removed));
    assert!(!absence.test_in(&context, &live_four));
    let no_defaults = Items::empty();
    assert!(
        parse(absence_source, &no_defaults)
            .test_in(&context, &ItemStack::new("minecraft:ominous_bottle", 1),)
    );
    for value in [Tag::Int(-1), Tag::Int(5), Tag::String("3".into())] {
        bottle
            .data_mut()
            .set_other("minecraft:ominous_bottle_amplifier", Some(value.clone()));
        assert!(!zero.test_in(&context, &bottle));
        assert!(!four.test_in(&context, &bottle));
        let patch = Tag::compound([("minecraft:ominous_bottle_amplifier", value)]);
        (_, bottle.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(bottle.data().unwrap().ominous_amplifier, None);
        assert!(!zero.test_in(&context, &bottle));
        assert!(!four.test_in(&context, &bottle));
    }

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":-1}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":5}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":"3"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":1e100}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:ominous_bottle_amplifier:"),
            "{error}"
        );
        assert!(Ingredient::parse(&json, &items).is_none());
        let refused = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_amplifier".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless",
                    "ingredients":[json],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert!(refused.crafting.is_empty());
        assert!(refused.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn potion_duration_scale_decodes_and_matches_float_values() {
    const ID: &str = "minecraft:potion_duration_scale";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:potion".into(),
        horizon_core::nbt::Compound::from_iter([(ID.into(), Tag::Double(1.5f64.to_bits()))]),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":1.5}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:potion", 1);
    assert!(positive.test_in(&context, &stack), "borrowed item default");
    for value in [
        Tag::Float(1.5f32.to_bits()),
        Tag::Double(1.50000000001f64.to_bits()),
    ] {
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(positive.test_in(&context, &stack), "floatValue rounding");
    }
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float(1.25f32.to_bits())));
    assert!(!positive.test_in(&context, &stack));

    let one = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":1}}"#,
        &items,
    );
    for value in [
        Tag::Byte(1),
        Tag::Short(1),
        Tag::Int(1),
        Tag::Long(1),
        Tag::Float(1.0f32.to_bits()),
        Tag::Double(1.0f64.to_bits()),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .unwrap(),
            Tag::Float(0x3f80_0000),
        );
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(one.test_in(&context, &stack));
    }
    let large = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":4294967296}}"#,
        &items,
    );
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Long(4_294_967_296)));
    assert!(
        large.test_in(&context, &stack),
        "boxed Long does not truncate"
    );

    let zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":0}}"#,
        &items,
    );
    // JsonOps converts the integral decimal -0.0 to a zero byte before decoding.
    let json_zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":-0.0}}"#,
        &items,
    );
    for value in [Tag::Byte(0), Tag::Double(f64::MIN_POSITIVE.to_bits())] {
        assert_eq!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .unwrap(),
            Tag::Float(0),
        );
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(zero.test_in(&context, &stack));
        assert!(json_zero.test_in(&context, &stack));
    }
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float((-0.0f32).to_bits())));
    assert!(
        !zero.test_in(&context, &stack),
        "Float.equals distinguishes zero signs"
    );

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":-1}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":"fast"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":{}}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none(), "{source}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
    }
    for value in [
        Tag::Float((-0.0f32).to_bits()),
        Tag::Double((-0.0f64).to_bits()),
        Tag::Double((-f64::MIN_POSITIVE).to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Float(0x7f80_0001),
        Tag::Double(f64::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Double(f64::MAX.to_bits()),
    ] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from_iter([(ID.into(), value)]),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
    }
    assert_eq!(
        horizon_command::item::decoded_component_value(
            ID,
            &Tag::Float(f32::MAX.to_bits()),
            &items.component_registry,
        )
        .unwrap(),
        Tag::Float(0x7f7f_ffff),
    );
    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"!minecraft:potion_duration_scale":{}}}"#;
    let absence = parse(absence_source, &items);
    let mut removed = ItemStack::new("minecraft:potion", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:potion_duration_scale",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!positive.test_in(&context, &removed));
    let mut populated = ItemStack::new("minecraft:potion", 1);
    populated
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float(1.5_f32.to_bits())));
    assert!(!absence.test_in(&context, &populated));
    assert!(
        parse(absence_source, &Items::empty())
            .test_in(&context, &ItemStack::new("minecraft:potion", 1),)
    );
}

#[test]
fn minimum_attack_charge_recipe_decodes_and_matches_float_values() {
    const ID: &str = "minecraft:minimum_attack_charge";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        horizon_core::nbt::Compound::from([(ID.into(), Tag::Double(0.35_f64.to_bits()))]),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipe = |ingredient: &Value| {
        crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:minimum_attack_charge".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient],
                    "result": {"id": "minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap()
    };
    let stack = |value: Tag| {
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.data_mut().set_loot_component(ID, &Some(value));
        sword
    };
    for (source, value, actual) in [
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0}}"#,
            0.0_f32,
            Tag::Short(0),
        ),
        // JsonOps-to-NBT numeric conversion canonicalizes this exact integer zero.
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":-0.0}}"#,
            0.0,
            Tag::Byte(0),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1}}"#,
            1.0,
            Tag::Long(1),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0.35}}"#,
            0.35,
            Tag::Float(0.35_f32.to_bits()),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1.00000001}}"#,
            1.0,
            Tag::Int(1),
        ),
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert_eq!(recipe(&json).crafting.len(), 1, "{source}");
        let ingredient = Ingredient::parse(&json, &items).unwrap();
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .other(ID),
            Some(&Tag::Float(value.to_bits())),
            "the published patch must contain the decoded float"
        );
        assert!(ingredient.test_in(&context, &stack(actual)), "{source}");
        assert!(
            !ingredient.test_in(&context, &stack(Tag::Double(0.75_f64.to_bits()))),
            "{source}"
        );
        assert_eq!(
            ingredient.test_in(&context, &ItemStack::new("minecraft:diamond_sword", 1)),
            value == 0.35,
            "the borrowed double-valued item default has the same float value"
        );
        for actual in [
            Tag::Byte(value as i8),
            Tag::Short(value as i16),
            Tag::Int(value as i32),
            Tag::Long(value as i64),
            Tag::Float(value.to_bits()),
            Tag::Double(f64::from(value).to_bits()),
        ] {
            if value == 0.0 || value == 1.0 {
                assert!(ingredient.test_in(&context, &stack(actual)));
            }
        }
    }
    let interior = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0.35}}"#,
        &items,
    );
    assert!(interior.test_in(&context, &stack(Tag::Double(0.350000001_f64.to_bits()))));
    assert!(!interior.test_in(&context, &stack(Tag::Double(0.3500001_f64.to_bits()))));
    let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
    let patch = Tag::compound([(
        "!minecraft:minimum_attack_charge",
        Tag::Compound(Default::default()),
    )]);
    (_, removed.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(!interior.test_in(&context, &removed));

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":-0.01}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1.01}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":"0.5"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":{}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":[]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1e300}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none(), "{source}");
        assert!(recipe(&json).crafting.is_empty(), "{source}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
    }
    // JSON cannot spell NaN or infinity. The same recipe validator must refuse
    // these numeric tags, including negative values that narrow to negative zero.
    for value in [
        Tag::Float(f32::NAN.to_bits()),
        Tag::Double(f64::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Float((-0.0_f32).to_bits()),
        Tag::Double((-0.0_f64).to_bits()),
        Tag::Double((-1e-300_f64).to_bits()),
    ] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([(ID.into(), value)]),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
        assert!(error.contains("within range [0.0;1.0]"), "{error}");
    }
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:iron_sword"],"components":{"!minecraft:minimum_attack_charge":{}}}"#,
        &items,
    );
    let positive = parse(
        r#"{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:iron_sword"],"components":{"minecraft:minimum_attack_charge":0.35}}"#,
        &items,
    );
    let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:minimum_attack_charge",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!positive.test_in(&context, &removed));
    assert!(!absence.test_in(&context, &stack(Tag::Float(0.35_f32.to_bits()))));
    let no_default = ItemStack::new("minecraft:iron_sword", 1);
    assert!(absence.test_in(&context, &no_default));
    assert!(!positive.test_in(&context, &no_default));
}

#[test]
fn use_cooldown_recipe_decodes_record_and_matches_borrowed_values() {
    const ID: &str = "minecraft:use_cooldown";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        horizon_core::nbt::Compound::from_iter([(
            ID.into(),
            Tag::compound([
                ("seconds", Tag::Int(2)),
                ("cooldown_group", Tag::String(":shared".into())),
                ("ignored", Tag::Int(99)),
            ]),
        )]),
    )]));
    let ingredient_json = |value: serde_json::Value| serde_json::json!({"type":"neoforge:components","items":"minecraft:stick","components":{(ID):value}});
    let compile = |ingredient: serde_json::Value| {
        let root = tempfile::tempdir().unwrap();
        crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:use_cooldown".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless",
                    "ingredients":[ingredient],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load(),
        )
        .unwrap()
    };
    let json = ingredient_json(serde_json::json!({
        "seconds":2.0,"cooldown_group":"shared","unknown":false
    }));
    let recipes = compile(json.clone());
    assert_eq!(recipes.crafting.len(), 1);
    let ingredient = Ingredient::parse(&json, &items).unwrap();
    let canonical = Tag::compound([
        ("seconds", Tag::Float(2.0f32.to_bits())),
        ("cooldown_group", Tag::String("minecraft:shared".into())),
    ]);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other(ID),
        Some(&canonical)
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:stick", 1);
    assert!(recipes.recipe_matches(&context, "fixture:use_cooldown", &[Some(stack.clone())], 1));
    for group in ["shared", ":shared", "minecraft:shared"] {
        stack.data_mut().set_other(
            ID,
            Some(Tag::compound([
                ("seconds", Tag::Double(2.0f64.to_bits())),
                ("cooldown_group", Tag::String(group.into())),
                ("ignored", Tag::String("different".into())),
            ])),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert!(recipes.recipe_matches(
            &context,
            "fixture:use_cooldown",
            &[Some(stack.clone())],
            1
        ));
    }
    for value in [
        Tag::compound([("seconds", Tag::Int(2))]),
        Tag::compound([
            ("seconds", Tag::Int(3)),
            ("cooldown_group", Tag::String("shared".into())),
        ]),
        Tag::compound([
            ("seconds", Tag::Int(2)),
            ("cooldown_group", Tag::String("fixture:shared".into())),
        ]),
    ] {
        stack.data_mut().set_other(ID, Some(value));
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:use_cooldown",
            &[Some(stack.clone())],
            1
        ));
    }
    stack.data_mut().set_other(ID, None);
    stack.data_mut().set_other(
        "!minecraft:use_cooldown",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!ingredient.test_in(&context, &stack));

    let decode = |value: &Tag| {
        horizon_command::item::decoded_component_value(ID, value, &items.component_registry)
    };
    for number in [
        Tag::Byte(2),
        Tag::Short(2),
        Tag::Int(2),
        Tag::Long(2),
        Tag::Float(2.0f32.to_bits()),
        Tag::Double(2.0f64.to_bits()),
    ] {
        assert_eq!(
            decode(&Tag::compound([("seconds", number)])).unwrap(),
            Tag::compound([("seconds", Tag::Float(2.0f32.to_bits()))])
        );
    }
    for (number, bits) in [
        (Tag::Long(16_777_217), 16_777_216.0f32.to_bits()),
        (Tag::Long(i64::MAX), (i64::MAX as f32).to_bits()),
        (
            Tag::Double(f64::from(f32::MAX).to_bits()),
            f32::MAX.to_bits(),
        ),
        (Tag::Float(1), 1),
    ] {
        assert_eq!(
            decode(&Tag::compound([("seconds", number)])).unwrap(),
            Tag::compound([("seconds", Tag::Float(bits))])
        );
    }
    for number in [
        Tag::Int(0),
        Tag::Int(-1),
        Tag::Long(i64::MIN),
        Tag::Float((-0.0f32).to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Double(f64::MAX.to_bits()),
        Tag::Double(f64::MIN_POSITIVE.to_bits()),
        Tag::String("2".into()),
    ] {
        assert!(decode(&Tag::compound([("seconds", number)])).is_err());
    }
    for units in [vec![0xd800], vec![0xdc00], vec![0xd83d, 0xde00]] {
        assert!(
            decode(&Tag::compound([
                ("seconds", Tag::Int(2)),
                (
                    "cooldown_group",
                    Tag::String(horizon_core::nbt::JavaString(units))
                ),
            ]))
            .is_err()
        );
    }
    for value in [
        serde_json::json!({}),
        serde_json::json!(2),
        serde_json::json!({"seconds":0}),
        serde_json::json!({"seconds":-1}),
        serde_json::json!({"seconds":1e100}),
        serde_json::json!({"seconds":"2"}),
        serde_json::json!({"seconds":2,"cooldown_group":7}),
        serde_json::json!({"seconds":2,"cooldown_group":"Minecraft:shared"}),
        serde_json::json!({"seconds":2,"cooldown_group":"bad group"}),
        serde_json::json!({"seconds":2,"cooldown_group":"a:b:c"}),
    ] {
        let json = ingredient_json(value);
        assert!(compile(json.clone()).crafting.is_empty(), "{json}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:use_cooldown:"),
            "{error}"
        );
    }
    let absent_group = ingredient_json(serde_json::json!({"seconds":2}));
    let no_group = compile(absent_group);
    assert_eq!(no_group.crafting.len(), 1);
    stack.data_mut().set_other("!minecraft:use_cooldown", None);
    stack
        .data_mut()
        .set_other(ID, Some(Tag::compound([("seconds", Tag::Int(2))])));
    assert!(no_group.recipe_matches(&context, "fixture:use_cooldown", &[Some(stack.clone())], 1));
    stack.data_mut().set_other(
        ID,
        Some(Tag::compound([
            ("seconds", Tag::Int(2)),
            ("cooldown_group", Tag::String("".into())),
        ])),
    );
    assert!(!no_group.recipe_matches(&context, "fixture:use_cooldown", &[Some(stack)], 1));
    for group in ["", ":", "minecraft:"] {
        assert_eq!(
            decode(&Tag::compound([
                ("seconds", Tag::Int(2)),
                ("cooldown_group", Tag::String(group.into())),
            ]))
            .unwrap(),
            Tag::compound([
                ("seconds", Tag::Float(2.0f32.to_bits())),
                ("cooldown_group", Tag::String("minecraft:".into())),
            ])
        );
    }
}

#[test]
fn use_cooldown_literal_float_bounds_and_missing_seconds() {
    let items = Items::empty();
    for seconds in [
        Tag::Float(0),
        Tag::Float((-0.0_f32).to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Int(-1),
    ] {
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:use_cooldown",
                &Tag::compound([("seconds", seconds)]),
                &items.component_registry
            )
            .is_err()
        );
    }
    for seconds in [f32::from_bits(1), f32::MAX] {
        let raw = Tag::compound([("seconds", Tag::Float(seconds.to_bits()))]);
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:use_cooldown",
                &raw,
                &items.component_registry
            )
            .unwrap(),
            raw
        );
    }
    assert!(
        horizon_command::item::decoded_component_value(
            "minecraft:use_cooldown",
            &Tag::compound([("cooldown_group", Tag::String("shared".into()))]),
            &items.component_registry
        )
        .is_err()
    );
}
