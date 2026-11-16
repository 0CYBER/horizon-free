use super::*;
#[test]
fn rollup_89_codecs_reject_bad_records_preserve_siblings_and_presence() {
    use serde_json::json;
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (id, valid, malformed) in [
        (
            "minecraft:cushion/color",
            json!("red"),
            vec![json!("RED"), json!("unknown")],
        ),
        (
            "minecraft:tropical_fish/base_color",
            json!("red"),
            vec![json!("RED"), json!("unknown")],
        ),
        (
            "minecraft:tropical_fish/pattern_color",
            json!("red"),
            vec![json!("RED"), json!("unknown")],
        ),
        (
            "minecraft:lodestone_tracker",
            json!({}),
            vec![
                json!({"target":{}}),
                json!({"target":{"dimension":"minecraft:overworld","pos":[1,2]}}),
                json!({"tracked":"true"}),
            ],
        ),
        (
            "minecraft:container_loot",
            json!({"loot_table":"fixture:unknown","seed":1e30}),
            vec![
                json!({}),
                json!({"loot_table":null}),
                json!({"loot_table":false}),
                json!({"loot_table":"BAD"}),
                json!({"loot_table":"fixture:unknown","seed":"1"}),
            ],
        ),
        (
            "minecraft:tooltip_display",
            json!({}),
            vec![
                json!({"hide_tooltip":"true"}),
                json!({"hidden_components":1}),
                json!({"hidden_components":[1]}),
                json!({"hidden_components":["fixture:unknown"]}),
            ],
        ),
    ] {
        let mut items = Items::empty();
        std::sync::Arc::make_mut(&mut items.component_registry)
            .persistent_components
            .push("minecraft:custom_data".into());
        let source = |payload| json!({"type":"neoforge:components","items":"minecraft:paper","components":{id:payload},"strict":true});
        for payload in [json!(null), json!(false), json!(true), json!(0), json!([])]
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
        for malformed in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
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
fn tooltip_default_prototype_and_optional_null_fields_match_defaults() {
    use serde_json::json;
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:paper".into(),
        horizon_core::nbt::Compound::from([(
            "minecraft:tooltip_display".into(),
            Tag::compound([] as [(&str, Tag); 0]),
        )]),
    )]));
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    for payload in [
        json!({}),
        json!({"hidden_components":[]}),
        json!({"hide_tooltip":null}),
        json!({"hidden_components":null}),
    ] {
        let ingredient = Ingredient::parse(&json!({"type":"neoforge:components","items":"minecraft:paper","components":{"minecraft:tooltip_display":payload}}), &items).unwrap();
        assert!(ingredient.test_in(&ctx, &ItemStack::new("minecraft:paper", 1)));
    }
}

#[test]
#[ignore = "daily"]
fn tooltip_hidden_set_accepts_a_real_transient_registry_type() {
    use serde_json::json;
    let items = resources();
    let source = json!({"type":"neoforge:components","items":"minecraft:paper","components":{"minecraft:tooltip_display":{"hidden_components":["minecraft:creative_slot_lock"]}}});
    components::validate_recipe_components(&source, &items).unwrap();
    let ingredient = Ingredient::parse(&source, &items).unwrap();
    let mut paper = ItemStack::new("minecraft:paper", 1);
    paper.data_mut().set_other(
        "minecraft:tooltip_display",
        Some(Tag::compound([(
            "hidden_components",
            Tag::List(vec![Tag::String("minecraft:creative_slot_lock".into())]),
        )])),
    );
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    assert!(ingredient.test_in(&ctx, &paper));
}

#[test]
fn tracker_and_container_optional_nulls_preserve_defaults_but_required_nulls_fail() {
    use serde_json::json;
    let items = Items::empty();
    let reg = &items.component_registry;
    for (id, json, expected) in [
        (
            "minecraft:lodestone_tracker",
            json!({"target":null}),
            Tag::Compound(Default::default()),
        ),
        (
            "minecraft:lodestone_tracker",
            json!({"tracked":null}),
            Tag::Compound(Default::default()),
        ),
        (
            "minecraft:lodestone_tracker",
            json!({"target":null,"tracked":null}),
            Tag::Compound(Default::default()),
        ),
        (
            "minecraft:container_loot",
            json!({"loot_table":"fixture:unknown","seed":null}),
            Tag::compound([
                ("loot_table", Tag::String("fixture:unknown".into())),
                ("seed", Tag::Long(0)),
            ]),
        ),
    ] {
        let source =
            json!({"type":"neoforge:components","items":"minecraft:paper","components":{id:json}});
        assert!(Ingredient::parse(&source, &items).is_some());
        let raw = if id == "minecraft:lodestone_tracker" {
            Tag::compound([("target", Tag::End), ("tracked", Tag::End)])
        } else {
            Tag::compound([
                ("loot_table", Tag::String("fixture:unknown".into())),
                ("seed", Tag::End),
            ])
        };
        assert_eq!(
            horizon_command::item::decoded_component_value(id, &raw, reg).unwrap(),
            expected
        );
    }
    for (id, raw) in [
        (
            "minecraft:container_loot",
            Tag::compound([("loot_table", Tag::End)]),
        ),
        (
            "minecraft:lodestone_tracker",
            Tag::compound([(
                "target",
                Tag::compound([
                    ("dimension", Tag::End),
                    ("pos", Tag::IntArray(vec![1, 2, 3])),
                ]),
            )]),
        ),
        (
            "minecraft:lodestone_tracker",
            Tag::compound([(
                "target",
                Tag::compound([
                    ("dimension", Tag::String("minecraft:overworld".into())),
                    ("pos", Tag::End),
                ]),
            )]),
        ),
    ] {
        assert!(horizon_command::item::decoded_component_value(id, &raw, reg).is_err());
    }
}
