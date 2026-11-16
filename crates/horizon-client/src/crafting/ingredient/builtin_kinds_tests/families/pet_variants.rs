use super::*;
#[test]
fn components_pet_variants_use_registry_keys_and_inherited_prototypes() {
    use horizon_client_core::registries::RegistryIds;
    let mut items = Items::empty();
    let registries = std::sync::Arc::make_mut(&mut items.component_registry);
    for (registry, ids) in [
        ("minecraft:cat_variant", vec!["minecraft:tabby"]),
        (
            "minecraft:cat_sound_variant",
            vec!["minecraft:classic", "minecraft:royal"],
        ),
        ("minecraft:wolf_variant", vec!["example:custom/wolf"]),
        (
            "minecraft:wolf_sound_variant",
            vec!["minecraft:classic", "minecraft:cute"],
        ),
    ] {
        registries.by_registry.insert(
            registry.into(),
            RegistryIds {
                ids: ids.into_iter().map(str::to_owned).collect(),
                tags: vec!["example:pets".into()],
            },
        );
    }
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        [("minecraft:cat/variant".into(), Tag::String("tabby".into()))]
            .into_iter()
            .collect(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","strict":true,"components":{"minecraft:cat/variant":":tabby","minecraft:cat/sound_variant":"classic","minecraft:wolf/variant":"example:custom/wolf","minecraft:wolf/sound_variant":":cute"}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut stone = ItemStack::new("minecraft:stone", 2);
    for (component, value) in [
        ("minecraft:cat/sound_variant", "minecraft:classic"),
        ("minecraft:wolf/variant", "example:custom/wolf"),
        ("minecraft:wolf/sound_variant", "cute"),
    ] {
        stone
            .data_mut()
            .set_other(component, Some(Tag::String(value.into())));
    }
    assert!(ingredient.test_in(&context, &stone));
    stone.data_mut().set_other(
        "minecraft:cat/sound_variant",
        Some(Tag::String("royal".into())),
    );
    assert!(!ingredient.test_in(&context, &stone));
    stone.data_mut().set_other(
        "minecraft:cat/sound_variant",
        Some(Tag::String("classic".into())),
    );
    stone.data_mut().set_other(
        "!minecraft:cat/variant",
        Some(Tag::Compound(Default::default())),
    );
    assert!(!ingredient.test_in(&context, &stone));
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:cat/variant":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, &stone));
    assert!(!absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    stone.data_mut().set_other("!minecraft:cat/variant", None);
    assert!(ingredient.test_in(&context, &stone));
    stone.count = 0;
    assert!(!ingredient.test_in(&context, &stone));
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:cat/variant"),
        Some(&Tag::String("minecraft:tabby".into())),
    );
    for invalid in [
        serde_json::json!("example:missing"),
        serde_json::json!("#example:pets"),
        serde_json::json!({"id":"tabby"}),
        serde_json::json!(1),
    ] {
        let recipe = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:stone",
            "components":{"minecraft:cat/variant":invalid},
        });
        assert!(Ingredient::parse(&recipe, &items).is_none());
    }
}
