use super::*;

#[test]
fn components_dyed_color_decodes_rgb_forms_and_matches_borrowed_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:leather_helmet".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:dyed_color":[1,0,0]})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let integer = parse(
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":-65536}}"#,
        &items,
    );
    let vector = parse(
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1.0,0.0,0.0]}}"#,
        &items,
    );
    assert_eq!(
        vector.shown.as_ref().unwrap()[0].data().unwrap().dyed_color,
        Some(-65536)
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let default = ItemStack::new("minecraft:leather_helmet", 1);
    let mut typed = default.clone();
    typed.data_mut().dyed_color = Some(-65536);
    let mut raw = default.clone();
    raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(0)])),
    );
    for ingredient in [&integer, &vector] {
        assert!(ingredient.test_in(&context, &default));
        assert!(ingredient.test_in(&context, &typed));
        assert!(ingredient.test_in(&context, &raw));
    }
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            for ingredient in [&integer, &vector] {
                for stack in [&default, &typed, &raw] {
                    assert!(std::hint::black_box(ingredient.test_in(&context, stack)));
                }
            }
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    // RGB equality retains the integer's alpha bits; rendering opacity is unrelated.
    typed.data_mut().dyed_color = Some(16711680);
    assert!(!integer.test_in(&context, &typed));
    typed.data_mut().dyed_color = Some(-16776961);
    assert!(!vector.test_in(&context, &typed));
    raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::String("not a color".into())),
    );
    assert!(!integer.test_in(&context, &raw));

    // All boxed number widths and NbtOps collection forms reach the same codec.
    for (value, expected) in [
        (Tag::Byte(-1), -1),
        (Tag::Short(-32768), -32768),
        (Tag::Int(305419896), 305419896),
        (Tag::Long(4294901760), -65536),
        (Tag::Float((-65536.5f32).to_bits()), -65536),
        (Tag::Double((-65536.5f64).to_bits()), -65536),
        (Tag::ByteArray(vec![1, 0, 0]), -65536),
        (Tag::IntArray(vec![1, 0, 0]), -65536),
        (Tag::LongArray(vec![1, 0, 0]), -65536),
        (
            Tag::List(vec![
                Tag::Double(0.5f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
            -8454144,
        ),
        // Channels have no 0..1 restriction, and floor before byte masking.
        (Tag::ByteArray(vec![255, 2, 0]), 0xff01_fe00u32 as i32),
        (
            Tag::List(vec![
                Tag::Float(f32::NAN.to_bits()),
                Tag::Float(f32::INFINITY.to_bits()),
                Tag::Float(f32::NEG_INFINITY.to_bits()),
            ]),
            0xff00_ff00u32 as i32,
        ),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:dyed_color",
                &value,
                &items.component_registry,
            )
            .unwrap(),
            Tag::Int(expected)
        );
        let patch = Tag::compound([("minecraft:dyed_color", value)]);
        let (_, data) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(data.unwrap().dyed_color, Some(expected));
    }

    for source in [
        r##"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":"#ff0000"}}"##,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,0]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,0,0,1]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,"red",0]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":{"rgb":16711680,"show_in_tooltip":true}}}"#,
    ] {
        let json: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none());
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:dyed_color"), "{error}");
    }
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:leather_helmet","minecraft:stick"],"components":{"!minecraft:dyed_color":{}}}"#,
        &items,
    );
    assert!(!absence.test_in(&context, &default));
    let mut populated_typed = default.clone();
    populated_typed.data_mut().dyed_color = Some(-65536);
    assert!(!absence.test_in(&context, &populated_typed));
    let mut populated_raw = default.clone();
    populated_raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(0)])),
    );
    assert!(!absence.test_in(&context, &populated_raw));
    let mut removed = default.clone();
    removed.data_mut().set_loot_component(
        "!minecraft:dyed_color",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!integer.test_in(&context, &removed));
    assert!(!vector.test_in(&context, &removed));
    assert!(absence.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
}

#[test]
fn dye_components_compile_recipes_match_live_values_and_preserve_defaults() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:red_dye".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:dye":"red"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:red_dye","components":{"minecraft:dye":"red"},"strict":true}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:dye".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[{
                    "type":"neoforge:components", "items":"minecraft:red_dye",
                    "components":{"minecraft:dye":"red"}, "strict":true
                }],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:red_dye", 1);
    assert!(
        ingredient.test_in(&context, &stack),
        "prototype dye default"
    );
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    stack
        .data_mut()
        .set_loot_component("minecraft:dye", &Some(Tag::String("red".into())));
    assert_eq!(
        stack.data().unwrap().other("minecraft:dye"),
        Some(&Tag::String("red".into()))
    );
    assert!(ingredient.test_in(&context, &stack));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    for value in [
        Tag::String("blue".into()),
        Tag::String("RED".into()),
        Tag::Int(14),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:dye", &Some(value));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    }
    stack.data_mut().set_loot_component("minecraft:dye", &None);
    stack
        .data_mut()
        .set_loot_component("!minecraft:dye", &Some(Tag::Compound(Default::default())));
    assert!(!ingredient.test_in(&context, &stack));
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:red_dye","components":{"!minecraft:dye":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &ItemStack::new("minecraft:red_dye", 1)));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:blue_dye", 1)));
    let stone_absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:dye":{}}}"#,
        &items,
    );
    assert!(stone_absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
}

#[test]
fn malformed_dye_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("RED"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:red"),
        serde_json::json!(14),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:dye":value});
        if !value.is_null() {
            let rule = IngredientRule::Components {
                patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
                removed: vec![],
                strict: false,
            };
            let error = components::validate(&rule, &items).unwrap_err();
            assert!(error.contains("component minecraft:dye:"), "{error}");
        }
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:red_dye", "components":patch
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_dye".into(),
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
fn sheep_color_components_compile_recipes_and_match_live_values() {
    let items = Items::empty();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:sheep_spawn_egg","components":{"minecraft:sheep/color":"light_blue"},"strict":true}"#,
        &items,
    );
    let loose = parse(
        r#"{"type":"neoforge:components","items":"minecraft:sheep_spawn_egg","components":{"minecraft:sheep/color":"light_blue"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:sheep_spawn_egg","components":{"!minecraft:sheep/color":{}}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:sheep_color".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[{
                    "type":"neoforge:components", "items":"minecraft:sheep_spawn_egg",
                    "components":{"minecraft:sheep/color":"light_blue"}, "strict":true
                }],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:sheep_spawn_egg", 1);
    assert!(!ingredient.test_in(&context, &stack));
    assert!(absent.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    let patch = Tag::compound([("minecraft:sheep/color", Tag::String("light_blue".into()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    assert_eq!(
        stack.data().unwrap().other("minecraft:sheep/color"),
        Some(&Tag::String("light_blue".into()))
    );
    assert!(ingredient.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_data", &Some(Tag::Int(1)));
    assert!(loose.test_in(&context, &stack));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_data", &None);
    for value in [
        Tag::String("light_gray".into()),
        Tag::String("LIGHT_BLUE".into()),
        Tag::Int(3),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:sheep/color", &Some(value));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    }
    stack
        .data_mut()
        .set_loot_component("minecraft:sheep/color", &None);
    stack.data_mut().set_loot_component(
        "!minecraft:sheep/color",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absent.test_in(&context, &stack));
    assert!(!ingredient.test_in(&context, &stack));

    // Items.registerSpawnEgg / Item.Properties.spawnEgg set ENTITY_DATA, not SHEEP_COLOR.
    // This explicit prototype is synthetic, exercising defaults rather than an entity's white color.
    let mut defaults = Items::empty();
    defaults.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:sheep_spawn_egg".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:sheep/color":"light_blue"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let default_ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:sheep_spawn_egg","components":{"minecraft:sheep/color":"light_blue"},"strict":true}"#,
        &defaults,
    );
    let default_absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:sheep_spawn_egg","components":{"!minecraft:sheep/color":{}}}"#,
        &defaults,
    );
    let plain = ItemStack::new("minecraft:sheep_spawn_egg", 1);
    assert!(default_ingredient.test_in(&context, &plain));
    assert!(!default_absent.test_in(&context, &plain));
    assert!(!default_ingredient.test_in(&context, &stack));
    assert!(default_absent.test_in(&context, &stack));
    let mut populated = plain.clone();
    populated
        .data_mut()
        .set_loot_component("minecraft:sheep/color", &Some(Tag::String("red".into())));
    assert!(!default_absent.test_in(&context, &populated));
    assert!(!default_ingredient.test_in(&context, &populated));
    populated
        .data_mut()
        .set_loot_component("minecraft:sheep/color", &None);
    populated.data_mut().set_loot_component(
        "!minecraft:sheep/color",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(default_absent.test_in(&context, &populated));
    assert!(!default_ingredient.test_in(&context, &populated));
}

#[test]
fn malformed_sheep_color_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [Tag::Byte(0), Tag::End, Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:sheep/color".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:sheep/color"), "{error}");
    }
    for value in [
        serde_json::json!("LIGHT_BLUE"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:light_blue"),
        serde_json::json!(3),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:sheep/color":value});
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:sheep_spawn_egg", "components":patch
        });
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:sheep/color"), "{error}");
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_sheep_color".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
                (
                    "fixture:valid_sibling".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:stick"],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid_sibling",
            &[Some(ItemStack::new("minecraft:stick", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .is_some()
        );
        assert!(
            recipes
                .craft(
                    &context,
                    &[Some(ItemStack::new("minecraft:sheep_spawn_egg", 1))],
                    1
                )
                .is_none()
        );
    }
}

#[test]
fn shulker_color_components_compile_match_and_distinguish_absence_from_defaults() {
    // DataComponents.SHULKER_COLOR uses DyeColor.CODEC, not the mob's uncolored sentinel.
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:shulker_spawn_egg".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:shulker/color":"purple"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    // Literal oracle names; do not derive expected acceptance from the decoder palette.
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let source = serde_json::json!({
            "type":"neoforge:components",
            "items":["minecraft:shulker_spawn_egg", "minecraft:stone"],
            "components":{"minecraft:shulker/color":color},
            "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:shulker_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        let mut stack = ItemStack::new("minecraft:shulker_spawn_egg", 1);
        assert_eq!(ingredient.test_in(&context, &stack), color == "purple");
        assert_eq!(
            recipes.craft(&context, &[Some(stack.clone())], 1).is_some(),
            color == "purple"
        );
        let mut no_default = ItemStack::new("minecraft:stone", 1);
        assert!(!ingredient.test_in(&context, &no_default));
        assert!(
            recipes
                .craft(&context, &[Some(no_default.clone())], 1)
                .is_none()
        );
        let value = Tag::String(color.into());
        // Exact lowercase names are already canonical: no ordinal or namespace normalization.
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:shulker/color",
                &value,
                &Default::default()
            )
            .unwrap(),
            value
        );
        for candidate in [&mut stack, &mut no_default] {
            candidate
                .data_mut()
                .set_loot_component("minecraft:shulker/color", &Some(value.clone()));
            assert_eq!(
                candidate.data().unwrap().other("minecraft:shulker/color"),
                Some(&value)
            );
            assert!(ingredient.test_in(&context, candidate));
            assert!(
                recipes
                    .craft(&context, &[Some(candidate.clone())], 1)
                    .is_some()
            );
        }
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
        assert!(
            !ingredient.test_in(&context, &stack),
            "strict rejects extra components"
        );
        let mut loose_source = source.clone();
        loose_source["strict"] = serde_json::json!(false);
        assert!(
            Ingredient::parse(&loose_source, &items)
                .unwrap()
                .test_in(&context, &stack)
        );
        for invalid in [Tag::String("PURPLE".into()), Tag::Int(-1), Tag::Int(10)] {
            no_default
                .data_mut()
                .set_loot_component("minecraft:shulker/color", &Some(invalid));
            assert!(!ingredient.test_in(&context, &no_default));
            assert!(
                recipes
                    .craft(&context, &[Some(no_default.clone())], 1)
                    .is_none()
            );
        }
        stack = ItemStack::new("minecraft:shulker_spawn_egg", 1);
        stack.data_mut().set_loot_component(
            "!minecraft:shulker/color",
            &Some(Tag::Compound(Default::default())),
        );
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let absent = parse(
            r#"{"type":"neoforge:components","items":["minecraft:shulker_spawn_egg","minecraft:stone"],"components":{"!minecraft:shulker/color":{}},"strict":true}"#,
            &items,
        );
        assert!(absent.test_in(&context, &stack));
        assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
        assert!(!absent.test_in(&context, &ItemStack::new("minecraft:shulker_spawn_egg", 1)));
    }
}

#[test]
fn malformed_shulker_color_payloads_omit_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [Tag::Byte(0), Tag::End, Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:shulker/color".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:shulker/color"), "{error}");
    }
    for value in [
        serde_json::json!("PURPLE"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:purple"),
        serde_json::json!(" purple"),
        serde_json::json!(""),
        serde_json::json!(-1),
        serde_json::json!(0),
        serde_json::json!(10),
        serde_json::json!(15),
        serde_json::json!(16),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:shulker/color":value});
        let source = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:shulker_spawn_egg", "components":patch
        });
        let error = components::validate_recipe_components(&source, &items).unwrap_err();
        assert!(error.contains("minecraft:shulker/color"), "{error}");
        assert!(Ingredient::parse(&source, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_shulker_color".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":[source],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
                (
                    "fixture:valid_sibling".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:stick"],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid_sibling",
            &[Some(ItemStack::new("minecraft:stick", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .is_some()
        );
        assert!(
            recipes
                .craft(
                    &context,
                    &[Some(ItemStack::new("minecraft:shulker_spawn_egg", 1))],
                    1
                )
                .is_none()
        );
    }
    for value in [
        Tag::End,
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        Tag::String(horizon_core::nbt::JavaString(vec![
            112, 117, 114, 112, 108, 101, 0xd800,
        ])),
        Tag::Byte(-1),
        Tag::Short(10),
        Tag::Long(10),
        Tag::Float(10.0f32.to_bits()),
        Tag::Double(10.0f64.to_bits()),
    ] {
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:shulker/color",
                &value,
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn cat_collar_components_compile_recipes_match_live_values_and_preserve_defaults() {
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    // Independent DyeColor.CODEC names, not values obtained from the decoder's palette.
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let mut items = Items::empty();
        items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
            "minecraft:stick".into(),
            crate::json_nbt::json_to_compound(
                serde_json::json!({"minecraft:cat/collar":color})
                    .as_object()
                    .unwrap(),
            )
            .unwrap(),
        )]));
        // Component identifiers normalize; color names themselves must remain exact.
        let source = serde_json::json!({
            "type":"neoforge:components", "items":["minecraft:stick","minecraft:paper"],
            "components":{"cat/collar":color}, "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:cat_collar".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        let mut stack = ItemStack::new("minecraft:stick", 1);
        assert!(
            ingredient.test_in(&context, &stack),
            "{color}: prototype default"
        );
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
        stack
            .data_mut()
            .set_loot_component("minecraft:cat/collar", &Some(Tag::String(color.into())));
        assert_eq!(
            stack.data().unwrap().other("minecraft:cat/collar"),
            Some(&Tag::String(color.into()))
        );
        assert!(ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        let other_color = if color == "red" { "blue" } else { "red" };
        for value in [
            Tag::String(other_color.into()),
            Tag::String("RED".into()),
            Tag::Int(14),
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:cat/collar", &Some(value));
            assert!(!ingredient.test_in(&context, &stack));
            assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        }
        stack
            .data_mut()
            .set_loot_component("minecraft:cat/collar", &Some(Tag::String(color.into())));
        stack
            .data_mut()
            .set_loot_component("minecraft:custom_data", &Some(Tag::Int(7)));
        assert!(
            !ingredient.test_in(&context, &stack),
            "strict extra component"
        );
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let mut relaxed_source = source.clone();
        relaxed_source["strict"] = serde_json::json!(false);
        let relaxed = Ingredient::parse(&relaxed_source, &items).unwrap();
        assert!(relaxed.test_in(&context, &stack));

        stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_loot_component(
            "!minecraft:cat/collar",
            &Some(Tag::Compound(Default::default())),
        );
        assert!(
            !ingredient.test_in(&context, &stack),
            "removal masks prototype"
        );
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let absent = parse(
            r#"{"type":"neoforge:components","items":["minecraft:stick","minecraft:paper"],"components":{"!minecraft:cat/collar":{}}}"#,
            &items,
        );
        assert!(absent.test_in(&context, &stack));
        assert!(!absent.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
        let mut paper = ItemStack::new("minecraft:paper", 1);
        assert!(absent.test_in(&context, &paper), "true absence");
        assert!(!ingredient.test_in(&context, &paper));
        assert!(recipes.craft(&context, &[Some(paper.clone())], 1).is_none());
        paper
            .data_mut()
            .set_loot_component("minecraft:cat/collar", &Some(Tag::String(color.into())));
        assert!(
            ingredient.test_in(&context, &paper),
            "explicit value without default"
        );
        assert!(!absent.test_in(&context, &paper));
        assert!(recipes.craft(&context, &[Some(paper.clone())], 1).is_some());
        paper.count = 0;
        assert!(!ingredient.test_in(&context, &paper));
        assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:air", 1)));
    }
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({
                "minecraft:cat/collar":"red", "minecraft:custom_data":{"extra":1}
            })
            .as_object()
            .unwrap(),
        )
        .unwrap(),
    )]));
    let strict = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:cat/collar":"red"},"strict":true}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    assert!(
        !strict.test_in(&context, &stack),
        "strict counts prototype extras"
    );
    let removal = Tag::compound([("!minecraft:custom_data", Tag::Compound(Default::default()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(
        strict.test_in(&context, &stack),
        "removed prototype extra is absent"
    );
}

#[test]
fn malformed_cat_collar_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [Tag::Byte(0), Tag::End, Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:cat/collar".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:cat/collar"), "{error}");
    }
    for value in [
        serde_json::json!("RED"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:red"),
        serde_json::json!("light blue"),
        serde_json::json!(""),
        serde_json::json!(14),
        serde_json::json!(14.0),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:cat/collar":value});
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:stick", "components":patch
        });
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:cat/collar"), "{error}");
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_cat_collar".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
                (
                    "fixture:valid_sibling".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:paper"],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid_sibling",
            &[Some(ItemStack::new("minecraft:paper", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:paper", 1))], 1)
                .is_some()
        );
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .is_none()
        );
    }
}

#[test]
fn wolf_collar_components_compile_recipes_match_live_values_and_preserve_defaults() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:wolf_spawn_egg".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:wolf/collar":"red"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    // DataComponents.WOLF_COLLAR uses DyeColor.CODEC: names remain exact strings.
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:wolf_spawn_egg","components":{"wolf/collar":"red"},"strict":true}"#,
        &items,
    );
    let loose = parse(
        r#"{"type":"neoforge:components","items":"minecraft:wolf_spawn_egg","components":{"minecraft:wolf/collar":"red"}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:wolf_collar".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[{
                    "type":"neoforge:components", "items":"minecraft:wolf_spawn_egg",
                    "components":{"wolf/collar":"red"}, "strict":true
                }],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:wolf_spawn_egg", 1);
    assert!(
        ingredient.test_in(&context, &stack),
        "prototype collar default"
    );
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    let patch = Tag::compound([("minecraft:wolf/collar", Tag::String("red".into()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    assert_eq!(
        stack.data().unwrap().other("minecraft:wolf/collar"),
        Some(&Tag::String("red".into()))
    );
    assert!(ingredient.test_in(&context, &stack));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stack
        .data_mut()
        .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(loose.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    for value in [
        Tag::String("blue".into()),
        Tag::String("RED".into()),
        Tag::Int(14),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:wolf/collar", &Some(value));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(!loose.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    }
    stack
        .data_mut()
        .set_loot_component("minecraft:wolf/collar", &None);
    stack.data_mut().set_loot_component(
        "!minecraft:wolf/collar",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(!ingredient.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    let absent = parse(
        r#"{"type":"neoforge:components","items":["minecraft:wolf_spawn_egg","minecraft:stone"],"components":{"!minecraft:wolf/collar":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &ItemStack::new("minecraft:wolf_spawn_egg", 1)));
    assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    let missing = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:wolf/collar":"red"}}"#,
        &items,
    );
    assert!(!missing.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    let mut populated = ItemStack::new("minecraft:stone", 1);
    populated
        .data_mut()
        .set_loot_component("minecraft:wolf/collar", &Some(Tag::String("red".into())));
    assert!(missing.test_in(&context, &populated));
    assert!(!absent.test_in(&context, &populated));
    // A removed unrelated inherited component no longer counts against strict matching.
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:wolf_spawn_egg".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({
                "minecraft:wolf/collar":"red", "minecraft:custom_data":{"extra":1}
            })
            .as_object()
            .unwrap(),
        )
        .unwrap(),
    )]));
    let strict_with_extra = parse(
        r#"{"type":"neoforge:components","items":"minecraft:wolf_spawn_egg","components":{"minecraft:wolf/collar":"red"},"strict":true}"#,
        &items,
    );
    let mut inherited = ItemStack::new("minecraft:wolf_spawn_egg", 1);
    assert!(!strict_with_extra.test_in(&context, &inherited));
    inherited.data_mut().set_loot_component(
        "!minecraft:custom_data",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(strict_with_extra.test_in(&context, &inherited));
    let mut empty = ItemStack::new("minecraft:wolf_spawn_egg", 0);
    empty
        .data_mut()
        .set_loot_component("minecraft:wolf/collar", &Some(Tag::String("red".into())));
    assert!(!ingredient.test_in(&context, &empty));
}

#[test]
fn malformed_wolf_collar_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [Tag::Byte(0), Tag::End, Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:wolf/collar".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:wolf/collar"), "{error}");
    }
    for value in [
        serde_json::json!("RED"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:red"),
        serde_json::json!("light blue"),
        serde_json::json!(""),
        serde_json::json!(14),
        serde_json::json!(14.0),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:wolf/collar":value});
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:wolf_spawn_egg", "components":patch
        });
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(error.contains("minecraft:wolf/collar"), "{error}");
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_wolf_collar".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
                (
                    "fixture:valid_sibling".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:paper"],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(recipes.recipe_matches(
            &context,
            "fixture:valid_sibling",
            &[Some(ItemStack::new("minecraft:paper", 1))],
            1
        ));
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:paper", 1))], 1)
                .is_some()
        );
        assert!(
            recipes
                .craft(
                    &context,
                    &[Some(ItemStack::new("minecraft:wolf_spawn_egg", 1))],
                    1
                )
                .is_none()
        );
    }
}

#[test]
fn cushion_color_components_compile_recipes_and_match_live_defaults() {
    // Items.CUSHION installs each color and max_stack_size=16. Unlike ItemInfo's
    // catalog hint, live StackData retains cushion/color as a string in `other`.
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let item = format!("minecraft:{color}_cushion");
        let mut items = Items::empty();
        items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
            item.clone(),
            crate::json_nbt::json_to_compound(
                serde_json::json!({
                    "minecraft:cushion/color":color, "minecraft:max_stack_size":16
                })
                .as_object()
                .unwrap(),
            )
            .unwrap(),
        )]));
        let source = serde_json::json!({
            "type":"neoforge:components", "items":item,
            "components":{"minecraft:cushion/color":color,"minecraft:max_stack_size":16},
            "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:cushion_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        let mut stack = ItemStack::new(item.as_str(), 1);
        assert!(ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
        let patch = Tag::compound([("minecraft:cushion/color", Tag::String(color.into()))]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(
            stack.data().unwrap().other("minecraft:cushion/color"),
            Some(&Tag::String(color.into()))
        );
        assert!(ingredient.test_in(&context, &stack));
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let mut loose_source = source.clone();
        loose_source["strict"] = serde_json::json!(false);
        assert!(
            Ingredient::parse(&loose_source, &items)
                .unwrap()
                .test_in(&context, &stack)
        );
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &None);
        for value in [
            Tag::String(if color == "red" { "blue" } else { "red" }.into()),
            Tag::String(color.to_uppercase().as_str().into()),
            Tag::Int(14),
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:cushion/color", &Some(value));
            assert!(!ingredient.test_in(&context, &stack));
            assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        }
        let removal = Tag::compound([(
            "!minecraft:cushion/color",
            Tag::Compound(Default::default()),
        )]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let absent_source = serde_json::json!({
            "type":"neoforge:components", "items":item,
            "components":{"!minecraft:cushion/color":{}}
        });
        let absent = Ingredient::parse(&absent_source, &items).unwrap();
        assert!(absent.test_in(&context, &stack));
        let plain = ItemStack::new(item.as_str(), 1);
        assert!(!absent.test_in(&context, &plain));
        let no_defaults = Items::empty();
        assert!(
            !Ingredient::parse(&source, &no_defaults)
                .unwrap()
                .test_in(&context, &plain)
        );
        assert!(
            Ingredient::parse(&absent_source, &no_defaults)
                .unwrap()
                .test_in(&context, &plain)
        );
        let mut incomplete_source = source.clone();
        incomplete_source["components"]
            .as_object_mut()
            .unwrap()
            .remove("minecraft:max_stack_size");
        assert!(
            !Ingredient::parse(&incomplete_source, &items)
                .unwrap()
                .test_in(&context, &plain)
        );
        assert!(!ingredient.test_in(&context, &ItemStack::new(item.as_str(), 0)));
        assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    }
}

#[test]
fn malformed_cushion_color_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("RED"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:red"),
        serde_json::json!(" red"),
        serde_json::json!(""),
        serde_json::json!(14),
        serde_json::json!(true),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:cushion/color":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:cushion/color:"),
            "{error}"
        );
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:red_cushion", "components":patch
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_cushion_color".into(),
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
fn tropical_fish_base_color_recipes_match_independent_colors_and_defaults() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:tropical_fish_bucket".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:tropical_fish/base_color":"light_blue"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    // Literal DyeColor names, independent of the decoder's firework palette.
    for name in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let source = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:tropical_fish_bucket",
            "components":{"minecraft:tropical_fish/base_color":name}, "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:tropical_fish_base_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        let mut stack = ItemStack::new("minecraft:tropical_fish_bucket", 1);
        assert_eq!(ingredient.test_in(&context, &stack), name == "light_blue");
        let patch = Tag::compound([(
            "minecraft:tropical_fish/base_color",
            Tag::String(name.into()),
        )]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(
            stack
                .data()
                .unwrap()
                .other("minecraft:tropical_fish/base_color"),
            Some(&Tag::String(name.into()))
        );
        assert!(ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        for value in [Tag::String("LIGHT_BLUE".into()), Tag::Int(3)] {
            stack
                .data_mut()
                .set_loot_component("minecraft:tropical_fish/base_color", &Some(value));
            assert!(!ingredient.test_in(&context, &stack));
            assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        }
    }
    let strict = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/base_color":"light_blue"},"strict":true}"#,
        &items,
    );
    let partial = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/base_color":"light_blue"}}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"!minecraft:tropical_fish/base_color":{}}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:tropical_fish_bucket", 1);
    assert!(strict.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &stack));
    stack.data_mut().set_loot_component(
        "minecraft:bucket_entity_data",
        &Some(Tag::compound([("BucketVariantTag", Tag::Int(917504))])),
    );
    assert!(
        partial.test_in(&context, &stack),
        "packed red cannot replace the blue default"
    );
    assert!(
        !strict.test_in(&context, &stack),
        "strict rejects extra bucket data"
    );
    let no_defaults = Items::empty();
    let independent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/base_color":"light_blue"}}"#,
        &no_defaults,
    );
    assert!(
        !independent.test_in(&context, &stack),
        "packed data cannot supply the component"
    );
    stack.data_mut().set_loot_component(
        "minecraft:tropical_fish/base_color",
        &Some(Tag::String("light_blue".into())),
    );
    assert!(independent.test_in(&context, &stack));
    stack.data_mut().set_loot_component(
        "minecraft:tropical_fish/base_color",
        &Some(Tag::String("red".into())),
    );
    assert!(
        !partial.test_in(&context, &stack),
        "explicit value overrides default"
    );
    let removal = Tag::compound([(
        "!minecraft:tropical_fish/base_color",
        Tag::Compound(Default::default()),
    )]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(!partial.test_in(&context, &stack));
    assert!(absent.test_in(&context, &stack));
    let absent_without_defaults = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"!minecraft:tropical_fish/base_color":{}}}"#,
        &no_defaults,
    );
    assert!(absent_without_defaults.test_in(
        &context,
        &ItemStack::new("minecraft:tropical_fish_bucket", 1),
    ));
}

#[test]
fn malformed_tropical_fish_base_colors_omit_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("LIGHT_BLUE"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:light_blue"),
        serde_json::json!(3),
        serde_json::json!(3.0),
        serde_json::json!(true),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:tropical_fish/base_color":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:tropical_fish/base_color:"),
            "{error}"
        );
        let source = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:tropical_fish_bucket",
            "components":patch
        });
        assert!(Ingredient::parse(&source, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_tropical_fish_base_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
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
fn tropical_fish_pattern_color_recipes_match_defaults_removals_and_distinct_fields() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:tropical_fish_bucket".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:tropical_fish/pattern_color":"light_blue"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/pattern_color":"light_blue"}}"#,
        &items,
    );
    let strict = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/pattern_color":"light_blue"},"strict":true}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"!minecraft:tropical_fish/pattern_color":{}}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:tropical_fish_pattern_color".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[{
                    "type":"neoforge:components", "items":"minecraft:tropical_fish_bucket",
                    "components":{"minecraft:tropical_fish/pattern_color":"light_blue"}
                }],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:tropical_fish_bucket", 1);
    assert!(ingredient.test_in(&context, &stack));
    assert!(strict.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    let patch = Tag::compound([
        (
            "minecraft:tropical_fish/pattern_color",
            Tag::String("light_blue".into()),
        ),
        (
            "minecraft:tropical_fish/base_color",
            Tag::String("red".into()),
        ),
        ("minecraft:base_color", Tag::String("yellow".into())),
        (
            "minecraft:bucket_entity_data",
            Tag::compound([("BucketVariantTag", Tag::Int(0))]),
        ),
    ]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    assert_eq!(
        stack
            .data()
            .unwrap()
            .other("minecraft:tropical_fish/pattern_color"),
        Some(&Tag::String("light_blue".into()))
    );
    assert_eq!(
        stack
            .data()
            .unwrap()
            .other("minecraft:tropical_fish/base_color"),
        Some(&Tag::String("red".into()))
    );
    assert!(ingredient.test_in(&context, &stack));
    assert!(
        !strict.test_in(&context, &stack),
        "unrequested extra components"
    );
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    for value in [
        Tag::String("red".into()),
        Tag::String("LIGHT_BLUE".into()),
        Tag::Int(3),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:tropical_fish/pattern_color", &Some(value));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    }
    let removal = Tag::compound([
        (
            "!minecraft:tropical_fish/pattern_color",
            Tag::Compound(Default::default()),
        ),
        (
            "minecraft:tropical_fish/base_color",
            Tag::String("light_blue".into()),
        ),
        (
            "minecraft:bucket_entity_data",
            Tag::compound([("BucketVariantTag", Tag::Int(3 << 24))]),
        ),
    ]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(absent.test_in(&context, &stack));
    assert!(recipes.craft(&context, &[Some(stack)], 1).is_none());

    let no_defaults = Items::empty();
    let plain_rule = parse(
        r#"{"type":"neoforge:components","items":"minecraft:tropical_fish_bucket","components":{"minecraft:tropical_fish/pattern_color":"light_blue"}}"#,
        &no_defaults,
    );
    let mut plain = ItemStack::new("minecraft:tropical_fish_bucket", 1);
    assert!(
        !plain_rule.test_in(&context, &plain),
        "absence is not a color default"
    );
    let unrelated = Tag::compound([
        (
            "minecraft:tropical_fish/base_color",
            Tag::String("light_blue".into()),
        ),
        ("minecraft:base_color", Tag::String("light_blue".into())),
        (
            "minecraft:bucket_entity_data",
            Tag::compound([("BucketVariantTag", Tag::Int(3 << 24))]),
        ),
    ]);
    (_, plain.extra) = crate::item_data::decode_components(Some(&unrelated));
    assert!(
        !plain_rule.test_in(&context, &plain),
        "other color encodings cannot substitute"
    );
}

#[test]
fn tropical_fish_pattern_color_accepts_all_canonical_names_without_normalizing_case() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        // Unqualified component keys normalize; DyeColor's exact names do not.
        let source = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:tropical_fish_bucket",
            "components":{"tropical_fish/pattern_color":color}, "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let mut stack = ItemStack::new("minecraft:tropical_fish_bucket", 1);
        stack.data_mut().set_loot_component(
            "minecraft:tropical_fish/pattern_color",
            &Some(Tag::String(color.into())),
        );
        assert!(ingredient.test_in(&context, &stack), "{color}");
    }
}

#[test]
fn malformed_tropical_fish_pattern_colors_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("LIGHT_BLUE"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:light_blue"),
        serde_json::json!(""),
        serde_json::json!(3),
        serde_json::json!(3.0),
        serde_json::json!(true),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:tropical_fish/pattern_color":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:tropical_fish/pattern_color:"),
            "{error}"
        );
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:tropical_fish_bucket",
            "components":patch
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_tropical_fish_pattern_color".into(),
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
