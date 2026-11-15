use super::*;
#[test]
fn rabbit_variant_recipe_codec_presence_and_borrowed_matching() {
    let compile = |items: &Items, patch: serde_json::Value, strict: bool| {
        let root = tempfile::tempdir().unwrap();
        crate::crafting::Recipes::from_sources(
            root.path(),
            items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:rabbit_variant".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [{
                        "type": "neoforge:components",
                        "items": "minecraft:rabbit_spawn_egg",
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
    // Rabbit.Variant.getSerializedName: DEFAULT=BROWN is a mob default, not
    // an item component. Items.registerSpawnEgg only adds ENTITY_DATA.
    for name in [
        "brown",
        "white",
        "black",
        "white_splotched",
        "gold",
        "salt",
        "evil",
    ] {
        let recipes = compile(&items, serde_json::json!({"rabbit/variant": name}), false);
        assert_eq!(recipes.crafting.len(), 1);
        let crate::crafting::Recipe::Shapeless { ingredients, .. } = &recipes.crafting[0].1 else {
            panic!("shapeless recipe expected");
        };
        let ingredient = &ingredients[0];
        let mut egg = ItemStack::new("minecraft:rabbit_spawn_egg", 1);
        assert!(!ingredient.test_in(&context, &egg));
        egg.data_mut()
            .set_other("minecraft:rabbit/variant", Some(Tag::String(name.into())));
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .other("minecraft:rabbit/variant"),
            Some(&Tag::String(name.into()))
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(std::hint::black_box(ingredient.test_in(&context, &egg)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        assert_eq!(
            recipes
                .craft(&context, &[Some(egg.clone())], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
        for value in [
            Tag::String(if name == "evil" { "brown" } else { "evil" }.into()),
            Tag::Int(99),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        ] {
            egg.data_mut()
                .set_other("minecraft:rabbit/variant", Some(value));
            assert!(!ingredient.test_in(&context, &egg));
            assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        }
    }
    for payload in [
        serde_json::json!(0),
        serde_json::json!(99),
        serde_json::json!(1.0),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!("BROWN"),
        serde_json::json!("minecraft:brown"),
        serde_json::json!("killer_bunny"),
        serde_json::json!(""),
    ] {
        let patch = serde_json::json!({"minecraft:rabbit/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        assert!(
            components::validate(&rule, &items)
                .unwrap_err()
                .contains("minecraft:rabbit/variant")
        );
        super::decoder_contracts::assert_malformed_decoder_reload_keeps_sibling(&items, patch);
    }
    let absent = compile(
        &items,
        serde_json::json!({"!minecraft:rabbit/variant": 99}),
        true,
    );
    let mut egg = ItemStack::new("minecraft:rabbit_spawn_egg", 1);
    assert!(absent.craft(&context, &[Some(egg.clone())], 1).is_some());
    // Synthetic item prototype tests the generic default path, not a vanilla
    // rabbit spawn default (the vanilla item has no RABBIT_VARIANT).
    let mut defaults = items.clone();
    defaults.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:rabbit_spawn_egg".into(),
        match Tag::compound([("minecraft:rabbit/variant", Tag::String("brown".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let exact = compile(
        &defaults,
        serde_json::json!({"minecraft:rabbit/variant": "brown"}),
        true,
    );
    let loose = compile(
        &defaults,
        serde_json::json!({"minecraft:rabbit/variant": "brown"}),
        false,
    );
    let omitted = compile(&defaults, serde_json::json!({}), true);
    let removal = compile(
        &defaults,
        serde_json::json!({"!minecraft:rabbit/variant": 99}),
        true,
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    egg.data_mut()
        .set_loot_component("minecraft:rabbit/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:rabbit/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:rabbit/variant",
        &Some(Tag::String("evil".into())),
    );
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
}
