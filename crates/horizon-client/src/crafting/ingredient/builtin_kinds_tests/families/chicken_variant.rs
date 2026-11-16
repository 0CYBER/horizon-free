use super::*;

#[test]
fn chicken_variant_decodes_registry_holders_and_matches_borrowed_values() {
    use horizon_client_core::registries::RegistryIds;
    use serde_json::json;

    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .by_registry
        .insert(
            "minecraft:chicken_variant".into(),
            RegistryIds {
                ids: vec!["minecraft:temperate".into(), "fixture:ashen".into()],
                tags: vec![],
            },
        );
    let source = |value| {
        json!({
            "type": "neoforge:components",
            "items": "minecraft:paper",
            "components": {"minecraft:chicken/variant": value},
            "strict": true,
        })
    };
    let ingredient = Ingredient::parse(&source(json!(":temperate")), &items).unwrap();
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:chicken/variant"),
        Some(&Tag::String("minecraft:temperate".into())),
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut paper = ItemStack::new("minecraft:paper", 1);
    paper.data_mut().set_other(
        "minecraft:chicken/variant",
        Some(Tag::String("temperate".into())),
    );
    assert!(ingredient.test_in(&context, &paper));
    paper.data_mut().set_other(
        "minecraft:chicken/variant",
        Some(Tag::String("fixture:ashen".into())),
    );
    assert!(!ingredient.test_in(&context, &paper));
    let custom = Ingredient::parse(&source(json!("fixture:ashen")), &items).unwrap();
    assert!(custom.test_in(&context, &paper));

    for invalid in [
        json!("fixture:missing"),
        json!("#fixture:variants"),
        json!("Temperate"),
        json!({"id": "temperate"}),
        json!(0),
        json!(null),
    ] {
        assert!(Ingredient::parse(&source(invalid), &items).is_none());
    }
    assert!(Ingredient::parse(&source(json!("temperate")), &Items::empty()).is_none());

    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:paper".into(),
        [(
            "minecraft:chicken/variant".into(),
            Tag::String("temperate".into()),
        )]
        .into_iter()
        .collect(),
    )]));
    let inherited = Ingredient::parse(&source(json!("temperate")), &items).unwrap();
    let mut plain = ItemStack::new("minecraft:paper", 1);
    assert!(inherited.test_in(&context, &plain));
    plain.data_mut().set_other(
        "!minecraft:chicken/variant",
        Some(Tag::Compound(Default::default())),
    );
    assert!(!inherited.test_in(&context, &plain));
    let absent = Ingredient::parse(
        &json!({
            "type": "neoforge:components",
            "items": "minecraft:paper",
            "components": {"!minecraft:chicken/variant": {}},
        }),
        &items,
    )
    .unwrap();
    assert!(absent.test_in(&context, &plain));
}
