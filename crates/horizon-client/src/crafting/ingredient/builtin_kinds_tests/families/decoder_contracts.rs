use super::*;
#[test]
fn rollup_90_codecs_reject_bad_records_preserve_siblings_and_presence() {
    use serde_json::json;
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (id, valid, malformed) in [
        (
            "minecraft:attack_animation",
            json!({}),
            vec![json!({"type": "BAD"}), json!({"duration": -1})],
        ),
        (
            "minecraft:damage_resistant",
            json!({"types": []}),
            vec![
                json!({}),
                json!({"types": null}),
                json!({"types": false}),
                json!({"types": [1]}),
            ],
        ),
        (
            "minecraft:use_effects",
            json!({}),
            vec![
                json!({"speed_multiplier": 1e40}),
                json!({"speed_multiplier": 1.0001}),
                json!({"can_sprint": "true"}),
            ],
        ),
        (
            "minecraft:attack_range",
            json!({}),
            vec![
                json!({"max_reach": 64.0001}),
                json!({"hitbox_margin": 1.0001}),
                json!({"mob_factor": 2.0001}),
            ],
        ),
        (
            "minecraft:rabbit/variant",
            json!("brown"),
            vec![json!("BAD"), json!("unknown")],
        ),
        (
            "minecraft:horse/variant",
            json!("white"),
            vec![json!("BAD"), json!("unknown")],
        ),
        (
            "minecraft:llama/variant",
            json!("creamy"),
            vec![json!("BAD"), json!("unknown")],
        ),
    ] {
        let mut items = Items::empty();
        std::sync::Arc::make_mut(&mut items.component_registry)
            .persistent_components
            .push("minecraft:custom_data".into());
        let source = |payload| json!({"type":"neoforge:components","items":"minecraft:paper","components":{id:payload},"strict":true});
        for payload in [
            json!(null),
            json!(false),
            json!(true),
            json!(0),
            json!([]),
            json!("malformed"),
        ]
        .into_iter()
        .chain(malformed)
        {
            let ingredient = source(payload);
            let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
            assert!(error.contains(id), "{error}");
            let recipe = |ingredient| json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
            let recipes = crate::crafting::Recipes::from_sources(
                &std::env::temp_dir(),
                &items,
                &horizon_world::loot::PackOverlay::NONE,
                vec![
                    ("fixture:invalid".into(), recipe(ingredient)),
                    ("fixture:valid".into(), recipe(json!("minecraft:stick"))),
                ],
                &load,
            )
            .unwrap();
            assert_eq!(recipes.crafting.len(), 1);
            assert!(
                recipes
                    .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                    .is_some()
            );
            assert!(
                recipes
                    .craft(&context, &[Some(ItemStack::new("minecraft:paper", 1))], 1)
                    .is_none()
            );
        }
        for value in [
            Tag::End,
            Tag::Byte(0),
            Tag::List(vec![]),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        ] {
            let rule = IngredientRule::Components {
                patch: horizon_core::nbt::Compound::from([(id.into(), value)]),
                removed: vec![],
                strict: false,
            };
            let error = components::validate(&rule, &items).unwrap_err();
            assert!(error.contains(id), "{error}");
        }
        let strict = Ingredient::parse(&source(valid.clone()), &items).unwrap();
        let mut loose_source = source(valid.clone());
        loose_source["strict"] = json!(false);
        let loose = Ingredient::parse(&loose_source, &items).unwrap();
        for malformed in [
            Tag::End,
            Tag::Byte(0),
            Tag::List(vec![]),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
            Tag::String("unknown".into()),
        ] {
            let mut candidate = ItemStack::new("minecraft:paper", 1);
            candidate.data_mut().set_other(id, Some(malformed));
            assert!(!loose.test_in(&context, &candidate), "{id}");
        }
        let removal_id = format!("!{id}");
        let absence_source = json!({"type":"neoforge:components","items":"minecraft:paper","components":{removal_id.clone():{}}});
        let absent = Ingredient::parse(&absence_source, &items).unwrap();
        let mut paper = ItemStack::new("minecraft:paper", 1);
        assert!(!strict.test_in(&context, &paper));
        assert!(absent.test_in(&context, &paper));
        let patch = crate::json_nbt::json_to_compound(
            source(valid.clone())["components"].as_object().unwrap(),
        )
        .unwrap();
        let raw = patch.values().next().unwrap().clone();
        paper.data_mut().set_other(id, Some(raw.clone()));
        assert!(strict.test_in(&context, &paper));
        assert!(!absent.test_in(&context, &paper));
        paper
            .data_mut()
            .set_other("minecraft:custom_data", Some(Tag::Int(1)));
        assert!(!strict.test_in(&context, &paper));
        assert!(loose.test_in(&context, &paper));
        items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
            "minecraft:paper".into(),
            patch,
        )]));
        let inherited = Ingredient::parse(&source(valid), &items).unwrap();
        let inherited_absent = Ingredient::parse(&absence_source, &items).unwrap();
        let mut paper = ItemStack::new("minecraft:paper", 1);
        assert!(inherited.test_in(&context, &paper));
        assert!(!inherited_absent.test_in(&context, &paper));
        paper.data_mut().set_other(id, Some(raw.clone()));
        assert!(inherited.test_in(&context, &paper));
        paper.data_mut().set_other(id, Some(Tag::Int(99)));
        assert!(!inherited.test_in(&context, &paper));
        paper.data_mut().set_other(id, Some(raw));
        paper.data_mut().set_loot_component(id, &None);
        paper
            .data_mut()
            .set_loot_component(&removal_id, &Some(Tag::Compound(Default::default())));
        assert!(!inherited.test_in(&context, &paper));
        assert!(inherited_absent.test_in(&context, &paper));
    }
}

#[test]
fn decoder_optional_nulls_equal_defaults_and_float_edges_are_literal() {
    use serde_json::json;
    let items = Items::empty();
    for (id, fields) in [
        ("minecraft:attack_animation", vec!["type", "duration"]),
        (
            "minecraft:use_effects",
            vec!["can_sprint", "interact_vibrations", "speed_multiplier"],
        ),
        (
            "minecraft:attack_range",
            vec![
                "min_reach",
                "max_reach",
                "min_creative_reach",
                "max_creative_reach",
                "hitbox_margin",
                "mob_factor",
            ],
        ),
    ] {
        let empty = Tag::Compound(Default::default());
        let expected =
            horizon_command::item::decoded_component_value(id, &empty, &items.component_registry)
                .unwrap();
        for field in fields {
            let raw = Tag::compound([(field, Tag::End)]);
            assert_eq!(
                horizon_command::item::decoded_component_value(id, &raw, &items.component_registry)
                    .unwrap(),
                expected
            );
            let source = json!({"type":"neoforge:components","items":"minecraft:paper","components":{id:{field:null}}});
            assert!(Ingredient::parse(&source, &items).is_some());
        }
    }
    for (id, field, limit) in [
        ("minecraft:use_effects", "speed_multiplier", 1.0f32),
        ("minecraft:attack_range", "max_reach", 64.0),
        ("minecraft:attack_range", "hitbox_margin", 1.0),
        ("minecraft:attack_range", "mob_factor", 2.0),
    ] {
        for number in [0.0, limit] {
            assert!(
                horizon_command::item::decoded_component_value(
                    id,
                    &Tag::compound([(field, Tag::Float(number.to_bits()))]),
                    &items.component_registry
                )
                .is_ok()
            );
        }
        for number in [-0.0f32, f32::NAN, f32::INFINITY, limit + 0.001] {
            assert!(
                horizon_command::item::decoded_component_value(
                    id,
                    &Tag::compound([(field, Tag::Float(number.to_bits()))]),
                    &items.component_registry
                )
                .is_err()
            );
        }
    }
    assert!(
        horizon_command::item::decoded_component_value(
            "minecraft:attack_range",
            &Tag::compound([("min_reach", Tag::Int(5)), ("max_reach", Tag::Int(1))]),
            &items.component_registry
        )
        .is_ok()
    );
}

#[test]
fn scalar_enum_unknown_names_have_component_named_diagnostics() {
    let items = Items::empty();
    for id in [
        "minecraft:rabbit/variant",
        "minecraft:horse/variant",
        "minecraft:llama/variant",
    ] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([(id.into(), Tag::String("unknown".into()))]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(id), "{error}");
        assert!(error.contains("Unknown element name:unknown"), "{error}");
    }
}

pub(super) fn assert_malformed_decoder_reload_keeps_sibling(
    items: &Items,
    patch: serde_json::Value,
) {
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    let ingredient =
        json!({"type":"neoforge:components","items":"minecraft:paper","components":patch});
    let recipe = |ingredient| json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![
            ("fixture:invalid".into(), recipe(ingredient)),
            ("fixture:sibling".into(), recipe(json!("minecraft:stick"))),
        ],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert!(
        recipes
            .craft(&ctx, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
            .is_some()
    );
    assert!(
        recipes
            .craft(&ctx, &[Some(ItemStack::new("minecraft:paper", 1))], 1)
            .is_none()
    );
}
