use super::*;
#[test]
fn pet_holder_contracts_preserve_siblings_removals_and_prototypes() {
    use serde_json::json;
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (id, valid, malformed) in [
        (
            "minecraft:cat/sound_variant",
            json!("fixture:valid"),
            vec![
                json!("fixture:missing"),
                json!("#fixture:tag"),
                json!({"id":"fixture:valid"}),
            ],
        ),
        (
            "minecraft:cat/variant",
            json!("fixture:valid"),
            vec![
                json!("fixture:missing"),
                json!("#fixture:tag"),
                json!({"id":"fixture:valid"}),
            ],
        ),
        (
            "minecraft:wolf/sound_variant",
            json!("fixture:valid"),
            vec![
                json!("fixture:missing"),
                json!("#fixture:tag"),
                json!({"id":"fixture:valid"}),
            ],
        ),
        (
            "minecraft:wolf/variant",
            json!("fixture:valid"),
            vec![
                json!("fixture:missing"),
                json!("#fixture:tag"),
                json!({"id":"fixture:valid"}),
            ],
        ),
    ] {
        let mut items = Items::empty();
        let registry = horizon_items::properties::pet_variants::registry(id).unwrap();
        std::sync::Arc::make_mut(&mut items.component_registry)
            .by_registry
            .insert(
                registry.into(),
                horizon_client_core::registries::RegistryIds {
                    ids: vec!["fixture:valid".into(), "fixture:other".into()],
                    tags: vec![],
                },
            );

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

        paper
            .data_mut()
            .set_other(id, Some(Tag::String("fixture:other".into())));
        assert!(!inherited.test_in(&context, &paper));
        assert!(!inherited_absent.test_in(&context, &paper));
        paper.data_mut().set_other(id, Some(raw));
        assert!(inherited.test_in(&context, &paper));

        paper.data_mut().set_loot_component(id, &None);
        paper
            .data_mut()
            .set_loot_component(&removal_id, &Some(Tag::Compound(Default::default())));
        assert!(!inherited.test_in(&context, &paper));
        assert!(inherited_absent.test_in(&context, &paper));
    }
}
