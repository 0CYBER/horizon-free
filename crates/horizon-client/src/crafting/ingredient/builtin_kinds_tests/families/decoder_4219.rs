use super::*;
#[test]
fn llama_variant_recipe_codec_preserves_names_defaults_removal_and_strictness() {
    let compile = |items: &Items, patch: serde_json::Value, strict: bool| {
        let root = tempfile::tempdir().unwrap();
        crate::crafting::Recipes::from_sources(
            root.path(),
            items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:llama_variant".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [{
                        "type": "neoforge:components",
                        "items": "minecraft:llama_spawn_egg",
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
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .persistent_components
        .push("minecraft:custom_data".into());
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    // Llama.Variant.CODEC names. Items.registerSpawnEgg/Properties.spawnEgg
    // supply entity_data, not LLAMA_VARIANT: the mob DEFAULT is not a prototype.
    for (name, different) in [
        ("creamy", "white"),
        ("white", "brown"),
        ("brown", "gray"),
        ("gray", "creamy"),
    ] {
        let recipes = compile(&items, serde_json::json!({"llama/variant": name}), false);
        assert_eq!(recipes.crafting.len(), 1);
        let crate::crafting::Recipe::Shapeless { ingredients, .. } = &recipes.crafting[0].1 else {
            panic!("shapeless recipe expected")
        };
        let ingredient = &ingredients[0];
        let mut egg = ItemStack::new("minecraft:llama_spawn_egg", 1);
        assert!(!ingredient.test_in(&context, &egg));
        egg.data_mut()
            .set_other("minecraft:llama/variant", Some(Tag::String(name.into())));
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
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        ] {
            egg.data_mut()
                .set_other("minecraft:llama/variant", Some(value));
            assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        }
    }
    for payload in [
        serde_json::json!(0),
        serde_json::json!(3),
        serde_json::json!(1.0),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!("CREAMY"),
        serde_json::json!("grey"),
        serde_json::json!("minecraft:creamy"),
        serde_json::json!(""),
    ] {
        let patch = serde_json::json!({"minecraft:llama/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.starts_with("component minecraft:llama/variant:"));
        super::decoder_contracts::assert_malformed_decoder_reload_keeps_sibling(&items, patch);
    }
    // Synthetic pack prototype exercises defaults without inventing a vanilla one.
    let mut prototype_items = items.clone();
    prototype_items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:llama_spawn_egg".into(),
        match Tag::compound([("minecraft:llama/variant", Tag::String("creamy".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let exact = compile(
        &prototype_items,
        serde_json::json!({"minecraft:llama/variant": "creamy"}),
        true,
    );
    let loose = compile(
        &prototype_items,
        serde_json::json!({"minecraft:llama/variant": "creamy"}),
        false,
    );
    let omitted = compile(&prototype_items, serde_json::json!({}), true);
    let removal = compile(
        &prototype_items,
        serde_json::json!({"!minecraft:llama/variant": 123}),
        true,
    );
    assert_eq!(removal.crafting.len(), 1);
    let mut egg = ItemStack::new("minecraft:llama_spawn_egg", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    egg.data_mut()
        .set_loot_component("minecraft:llama/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:llama/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:llama/variant",
        &Some(Tag::String("white".into())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.count = 0;
    assert!(loose.craft(&context, &[Some(egg)], 1).is_none());
    let absent = compile(
        &items,
        serde_json::json!({"!minecraft:llama/variant": {}}),
        true,
    );
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:llama_spawn_egg", 1))],
                1,
            )
            .is_some()
    );
}
