use super::*;
#[test]
fn attack_animation_codec_matches_recipes_and_rejects_malformed_siblings() {
    use horizon_core::nbt::Compound;
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    const ID: &str = "minecraft:attack_animation";
    fn record(kind: &str, duration: Tag) -> Tag {
        Tag::Compound(
            [
                ("type".into(), Tag::String(kind.into())),
                ("duration".into(), duration),
            ]
            .into_iter()
            .collect(),
        )
    }
    fn stack(value: Tag) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 3);
        stack.data_mut().other.push((ID.into(), value));
        stack
    }
    let ingredient = |patch: Value, strict| {
        json!({"type":"neoforge:components","items":"minecraft:diamond_sword",
            "components":patch,"strict":strict})
    };
    let recipe = |ingredient: Value| {
        json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],
            "result":{"id":"minecraft:diamond"}})
    };
    let mut items = Items::empty();
    // Synthetic test prototype, not a claim about vanilla sword defaults.
    items.component_defaults = Arc::new(HashMap::from([(
        "minecraft:diamond_sword".into(),
        [(ID.into(), record("whack", Tag::Short(6)))]
            .into_iter()
            .collect::<Compound>(),
    )]));
    let valid = [
        (json!({}), "whack", 6),
        (json!({"type":"whack"}), "whack", 6),
        (json!({"duration":6}), "whack", 6),
        (json!({"type":"none"}), "none", 6),
        (json!({"type":"stab"}), "stab", 6),
        (json!({"duration":0}), "whack", 0),
        (json!({"duration":-0.75}), "whack", 0),
        (json!({"duration":6.75}), "whack", 6),
        (json!({"duration":4294967302_i64}), "whack", 6),
        (json!({"duration":2147483647}), "whack", i32::MAX),
        (json!({"duration":1e100}), "whack", i32::MAX),
        (json!({"duration":-0.0}), "whack", 0),
        (json!({"duration":true}), "whack", 1),
        (json!({"duration":false}), "whack", 0),
        (
            json!({"type":"stab","duration":1,"ignored":[1,2]}),
            "stab",
            1,
        ),
        (json!({"type":null}), "whack", 6),
        (json!({"duration":null}), "whack", 6),
    ];
    let invalid = [
        json!([]),
        json!("whack"),
        json!(6),
        json!(null),
        json!(true),
        json!({"type":"WHACK"}),
        json!({"type":"minecraft:whack"}),
        json!({"type":"unknown"}),
        json!({"type":1}),
        json!({"type":{}}),
        json!({"duration":-1}),
        json!({"duration":-1.75}),
        json!({"duration":2147483648_i64}),
        json!({"duration":-1e100}),
        json!({"duration":"6"}),
        json!({"duration":"fixture:duration"}),
        json!({"duration":[]}),
        json!({"duration":{}}),
    ];
    let mut sources = Vec::new();
    for (index, (value, _, _)) in valid.iter().enumerate() {
        sources.push((
            format!("fixture:valid_{index}"),
            recipe(ingredient(json!({(ID):value}), false)),
        ));
    }
    for (index, value) in invalid.iter().enumerate() {
        sources.push((
            format!("fixture:invalid_{index}"),
            recipe(ingredient(json!({(ID):value}), false)),
        ));
    }
    sources.extend([
        ("fixture:control".into(), recipe(json!("minecraft:apple"))),
        (
            "fixture:strict".into(),
            recipe(ingredient(json!({(ID):{}}), true)),
        ),
        (
            "fixture:removed".into(),
            recipe(ingredient(json!({"!minecraft:attack_animation":{}}), false)),
        ),
    ]);
    let load = load();
    let ctx = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    let matches = |id: &str, stack: &ItemStack| {
        recipes.recipe_matches(&ctx, id, &[Some(stack.clone()), None, None, None], 2)
    };
    for (index, (_, kind, duration)) in valid.iter().enumerate() {
        let id = format!("fixture:valid_{index}");
        assert!(
            matches(&id, &stack(record(kind, Tag::Int(*duration)))),
            "{id}"
        );
        assert!(
            !matches(
                &id,
                &stack(record(kind, Tag::Int(duration.wrapping_sub(1))))
            ),
            "{id}"
        );
        let different = if *kind == "stab" { "whack" } else { "stab" };
        assert!(
            !matches(&id, &stack(record(different, Tag::Int(*duration)))),
            "{id}"
        );
    }
    for index in 0..invalid.len() {
        let id = format!("fixture:invalid_{index}");
        assert!(
            !recipes.crafting.iter().any(|(name, _)| name == &id),
            "{id}"
        );
        assert!(
            recipes
                .by_type
                .values()
                .all(|entries| entries.iter().all(|entry| entry.id != id)),
            "{id}"
        );
    }
    let (_, result) = recipes
        .find(
            &ctx,
            &[Some(ItemStack::new("minecraft:apple", 1)), None, None, None],
            2,
        )
        .unwrap();
    assert_eq!(result.item.as_ref(), "minecraft:diamond");
    let inherited = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(matches("fixture:valid_0", &inherited));
    assert!(matches("fixture:strict", &inherited));
    assert!(!matches("fixture:removed", &inherited));
    let mut extra = inherited.clone();
    extra
        .data_mut()
        .other
        .push(("minecraft:rarity".into(), Tag::String("rare".into())));
    assert!(matches("fixture:valid_0", &extra));
    assert!(!matches("fixture:strict", &extra));
    let mut removed = inherited.clone();
    removed.data_mut().other.push((
        "!minecraft:attack_animation".into(),
        Tag::Compound(Compound::new()),
    ));
    assert!(!matches("fixture:valid_0", &removed));
    assert!(matches("fixture:removed", &removed));
    assert!(!matches(
        "fixture:valid_0",
        &ItemStack::new("minecraft:apple", 1)
    ));
    assert!(!matches(
        "fixture:valid_0",
        &ItemStack::new("minecraft:diamond_sword", 0)
    ));
    let numeric = [
        (Tag::Byte(6), 6),
        (Tag::Short(6), 6),
        (Tag::Int(6), 6),
        (Tag::Long(4294967302), 6),
        (Tag::Float(6.75_f32.to_bits()), 6),
        (Tag::Double(6.75_f64.to_bits()), 6),
        (Tag::Float((-0.0_f32).to_bits()), 0),
        (Tag::Double((-0.75_f64).to_bits()), 0),
        (Tag::Float(f32::NAN.to_bits()), 0),
        (Tag::Double(f64::NAN.to_bits()), 0),
        (Tag::Float(f32::INFINITY.to_bits()), i32::MAX),
        (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
    ];
    for (number, expected) in numeric {
        let value = record("whack", number);
        assert_eq!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .unwrap(),
            record("whack", Tag::Int(expected)),
        );
        let id = match expected {
            0 => "fixture:valid_5",
            6 => "fixture:valid_0",
            _ => "fixture:valid_9",
        };
        assert!(matches(id, &stack(value)));
    }
    for number in [
        Tag::Byte(-1),
        Tag::Short(-1),
        Tag::Int(-1),
        Tag::Long(2147483648),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Double(f64::NEG_INFINITY.to_bits()),
    ] {
        let value = record("whack", number);
        assert!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .is_err()
        );
        assert!(!matches("fixture:valid_0", &stack(value)));
    }
    let compiled = Ingredient::parse(&ingredient(json!({(ID):{}}), false), &items).unwrap();
    let same = stack(record("whack", Tag::Double(6.9_f64.to_bits())));
    let different = stack(record("stab", Tag::Int(6)));
    let (_, counts) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(compiled.test_in(&ctx, std::hint::black_box(&same)));
            assert!(compiled.test_in(&ctx, std::hint::black_box(&inherited)));
            assert!(!compiled.test_in(&ctx, std::hint::black_box(&different)));
            assert!(!compiled.test_in(&ctx, std::hint::black_box(&removed)));
        }
    });
    assert_eq!(counts.calls, 0, "{counts:?}");
    assert_eq!(counts.bytes, 0, "{counts:?}");
}
