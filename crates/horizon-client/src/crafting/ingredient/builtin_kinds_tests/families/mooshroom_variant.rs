use super::*;

#[test]
fn mooshroom_variant_recipe_codec_preserves_names_defaults_removal_and_strictness() {
    // DataComponents.MOOSHROOM_VARIANT / MushroomCow.Variant.CODEC: red, brown.
    // Items.registerSpawnEgg and Item.Properties.spawnEgg do not set this component.
    let compile = |items: &Items, patch: serde_json::Value, strict: bool| {
        let root = tempfile::tempdir().unwrap();
        crate::crafting::Recipes::from_sources(
            root.path(),
            items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:mooshroom".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [{
                        "type": "neoforge:components",
                        "items": "minecraft:mooshroom_spawn_egg",
                        "components": patch,
                        "strict": strict
                    }],
                    "result": {"id": "minecraft:diamond"}
                }),
            )],
            &load(),
        )
        .unwrap()
    };
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (name, different) in [("red", "brown"), ("brown", "red")] {
        let recipes = compile(
            &items,
            serde_json::json!({"mooshroom/variant": name}),
            false,
        );
        assert_eq!(recipes.crafting.len(), 1);
        let ingredient = match &recipes.crafting[0].1 {
            crate::crafting::Recipe::Shapeless { ingredients, .. } => &ingredients[0],
            _ => panic!("shapeless recipe expected"),
        };
        let mut egg = ItemStack::new("minecraft:mooshroom_spawn_egg", 1);
        assert!(!ingredient.test_in(&context, &egg));
        egg.data_mut().set_other(
            "minecraft:mooshroom/variant",
            Some(Tag::String(name.into())),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(std::hint::black_box(ingredient.test_in(&context, &egg)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_some());
        for value in [
            Tag::String(different.into()),
            Tag::Int(0),
            Tag::End,
            Tag::Byte(0),
            Tag::Byte(1),
            Tag::List(vec![]),
            Tag::Compound(Default::default()),
            Tag::String(horizon_core::nbt::JavaString(vec![114, 101, 100, 0xd800])),
        ] {
            egg.data_mut()
                .set_other("minecraft:mooshroom/variant", Some(value));
            assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        }
    }
    for payload in [
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(1.0),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!("RED"),
        serde_json::json!("snow"),
        serde_json::json!("minecraft:red"),
        serde_json::json!(""),
    ] {
        let patch = serde_json::json!({"minecraft:mooshroom/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.starts_with("component minecraft:mooshroom/variant: "));
        let recipes = compile(&items, patch, false);
        assert!(recipes.crafting.is_empty());
        assert!(recipes.by_type.values().all(|entries| entries.is_empty()));
    }
    // A pack-supplied prototype exercises inheritance independently of vanilla's
    // absent item default (MushroomCow.Variant.DEFAULT is only the mob default).
    let mut defaults = Items::empty();
    std::sync::Arc::make_mut(&mut defaults.component_registry)
        .by_registry
        .entry("minecraft:data_component_type".into())
        .or_default()
        .ids
        .push("minecraft:custom_data".into());
    defaults.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:mooshroom_spawn_egg".into(),
        match Tag::compound([("minecraft:mooshroom/variant", Tag::String("brown".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let exact = compile(
        &defaults,
        serde_json::json!({"mooshroom/variant": "brown"}),
        true,
    );
    let loose = compile(
        &defaults,
        serde_json::json!({"mooshroom/variant": "brown"}),
        false,
    );
    let omitted = compile(&defaults, serde_json::json!({}), true);
    let removal = compile(
        &defaults,
        serde_json::json!({"!mooshroom/variant": 123}),
        true,
    );
    let mut egg = ItemStack::new("minecraft:mooshroom_spawn_egg", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    for (value, matches_default) in [("brown", true), ("red", false)] {
        egg.data_mut().set_loot_component(
            "minecraft:mooshroom/variant",
            &Some(Tag::String(value.into())),
        );
        assert_eq!(
            exact.craft(&context, &[Some(egg.clone())], 1).is_some(),
            matches_default
        );
        assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    }
    egg.data_mut()
        .set_loot_component("minecraft:mooshroom/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:mooshroom/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    let removed = Tag::compound([(
        "!minecraft:mooshroom/variant",
        Tag::compound([] as [(&str, Tag); 0]),
    )]);
    (_, egg.extra) = crate::item_data::decode_components(Some(&removed));
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:mooshroom/variant",
        &Some(Tag::String("red".into())),
    );
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
    let absent = compile(&items, serde_json::json!({"!mooshroom/variant": {}}), true);
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:mooshroom_spawn_egg", 1))],
                1,
            )
            .is_some()
    );
}

#[test]
fn mooshroom_variant_boundary_refusals_keep_a_valid_sibling() {
    const ID: &str = "minecraft:mooshroom/variant";
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for value in [
        Tag::End,
        Tag::Byte(0),
        Tag::Byte(1),
        Tag::Short(0),
        Tag::Long(0),
        Tag::Float(0.0_f32.to_bits()),
        Tag::Double(0.0_f64.to_bits()),
        Tag::List(vec![]),
        Tag::Compound(Default::default()),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
    ] {
        let rule = IngredientRule::Components {
            patch: [(ID.into(), value)].into_iter().collect(),
            removed: vec![],
            strict: false,
        };
        assert!(
            components::validate(&rule, &items)
                .unwrap_err()
                .starts_with(&format!("component {ID}: "))
        );
    }
    let unknown = IngredientRule::Components {
        patch: [(ID.into(), Tag::String("unknown".into()))]
            .into_iter()
            .collect(),
        removed: vec![],
        strict: false,
    };
    assert_eq!(
        components::validate(&unknown, &items),
        Err(format!("component {ID}: Unknown element name:unknown"))
    );
    for payload in [
        serde_json::json!(null),
        serde_json::json!(false),
        serde_json::json!(true),
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(1.0),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!("unknown"),
        serde_json::json!("arctic"),
        serde_json::json!("snow"),
        serde_json::json!(""),
        serde_json::json!("minecraft:brown"),
        serde_json::json!("BROWN"),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components", "items":"minecraft:mooshroom_spawn_egg", "components":{(ID):payload}});
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let root = tempfile::tempdir().unwrap();
        let recipes = crate::crafting::Recipes::from_sources(root.path(), &items, &horizon_world::loot::PackOverlay::NONE, vec![
            ("fixture:bad".into(), serde_json::json!({"type":"minecraft:crafting_shapeless", "ingredients":[ingredient], "result":{"id":"minecraft:diamond"}})),
            ("fixture:good".into(), serde_json::json!({"type":"minecraft:crafting_shapeless", "ingredients":["minecraft:stick"], "result":{"id":"minecraft:emerald"}})),
        ], &load).unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert_eq!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:emerald"
        );
    }
    let whole_null = serde_json::json!({"type":"neoforge:components", "items":"minecraft:mooshroom_spawn_egg", "components":null});
    assert!(Ingredient::parse(&whole_null, &items).is_none());
}
