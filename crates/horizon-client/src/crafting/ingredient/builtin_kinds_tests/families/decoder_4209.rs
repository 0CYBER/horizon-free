use super::*;
#[test]
fn use_effects_component_recipe_decodes_defaults_bounds_and_matches() {
    use crate::crafting::{Recipes, ResolvedRecipe, ResolvedSources};
    use horizon_world::loot::PackOverlay;
    use std::sync::Arc;

    let mut items = Items::empty();
    items.component_defaults = Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:use_effects":{}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let valid = [
        ("defaults", r#"{"use_effects":{}}"#, false),
        (
            "explicit",
            r#"{"minecraft:use_effects":{"can_sprint":false,"interact_vibrations":true,"speed_multiplier":0.2,"unknown":"ignored"}}"#,
            true,
        ),
        (
            "zero",
            r#"{"minecraft:use_effects":{"speed_multiplier":0}}"#,
            false,
        ),
        (
            "one",
            r#"{"minecraft:use_effects":{"can_sprint":0.5,"interact_vibrations":0,"speed_multiplier":1}}"#,
            false,
        ),
        ("removed", r#"{"!minecraft:use_effects":{}}"#, false),
    ];
    let invalid = [
        "false",
        "[]",
        r#""text""#,
        r#"{"can_sprint":"true"}"#,
        r#"{"interact_vibrations":{}}"#,
        r#"{"speed_multiplier":"0.2"}"#,
        r#"{"speed_multiplier":-0.01}"#,
        r#"{"speed_multiplier":1.001}"#,
    ];
    let recipe = |name: &str, patch: &str, strict: bool| {
        let file = Arc::new(
            serde_json::from_str::<serde_json::Value>(&format!(
                r#"{{"type":"minecraft:crafting_shaped","pattern":["A"],"key":{{"A":{{"type":"neoforge:components","items":"minecraft:stick","components":{patch},"strict":{strict}}}}},"result":{{"id":"minecraft:diamond"}}}}"#,
            ))
            .unwrap(),
        );
        ResolvedRecipe {
            name: format!("fixture:{name}"),
            resolved: file.clone(),
            file,
        }
    };
    let mut sources = ResolvedSources::default();
    for (name, patch, strict) in valid {
        sources.recipes.push(recipe(name, patch, strict));
    }
    for (index, payload) in invalid.iter().enumerate() {
        let patch = format!(r#"{{"minecraft:use_effects":{payload}}}"#);
        sources
            .recipes
            .push(recipe(&format!("invalid_{index}"), &patch, false));
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(
                serde_json::from_str::<serde_json::Value>(&patch)
                    .unwrap()
                    .as_object()
                    .unwrap(),
            )
            .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:use_effects:"),
            "{error}"
        );
    }
    let recipes =
        Recipes::from_resolved(&std::env::temp_dir(), &items, &PackOverlay::NONE, &sources)
            .unwrap();
    for (index, _) in invalid.iter().enumerate() {
        assert!(
            recipes
                .recipe(&format!("fixture:invalid_{index}"))
                .is_none()
        );
    }
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let matches = |name: &str, stack: &ItemStack| {
        recipes.recipe_matches(
            &context,
            &format!("fixture:{name}"),
            &[Some(stack.clone())],
            1,
        )
    };
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"use_effects":{}}}"#,
        &items,
    );
    let decoded = ingredient.shown.as_ref().unwrap()[0]
        .data()
        .unwrap()
        .other("minecraft:use_effects")
        .unwrap();
    assert_eq!(decoded.get("can_sprint"), Some(&Tag::Byte(0)));
    assert_eq!(decoded.get("interact_vibrations"), Some(&Tag::Byte(1)));
    assert_eq!(
        decoded.get("speed_multiplier"),
        Some(&Tag::Float(0.2_f32.to_bits())),
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    for payload in [
        None,
        Some(serde_json::json!({})),
        Some(serde_json::json!({
            "can_sprint":0, "interact_vibrations":-2.5,
            "speed_multiplier":0.200000001, "unused":[1,2,3]
        })),
    ] {
        let tag = payload.map(|value| crate::json_nbt::json_to_tag(&value).unwrap());
        stack
            .data_mut()
            .set_loot_component("minecraft:use_effects", &tag);
        assert!(matches("defaults", &stack));
        assert!(matches("explicit", &stack));
        assert!(!matches("removed", &stack));
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
    }
    for payload in [
        serde_json::json!({"can_sprint":true}),
        serde_json::json!({"interact_vibrations":false}),
        serde_json::json!({"speed_multiplier":0.3}),
        serde_json::json!({"can_sprint":"invalid"}),
    ] {
        let tag = crate::json_nbt::json_to_tag(&payload).unwrap();
        stack
            .data_mut()
            .set_loot_component("minecraft:use_effects", &Some(tag));
        assert!(!matches("defaults", &stack));
    }
    for (name, payload) in [
        ("zero", serde_json::json!({"speed_multiplier":0})),
        (
            "one",
            serde_json::json!({"can_sprint":2,"interact_vibrations":0,"speed_multiplier":1.0}),
        ),
    ] {
        let tag = crate::json_nbt::json_to_tag(&payload).unwrap();
        stack
            .data_mut()
            .set_loot_component("minecraft:use_effects", &Some(tag));
        assert!(matches(name, &stack));
        assert!(!matches("defaults", &stack));
    }
    stack
        .data_mut()
        .set_loot_component("minecraft:use_effects", &None);
    stack
        .data_mut()
        .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
    assert!(matches("defaults", &stack));
    assert!(
        !matches("explicit", &stack),
        "strict rejects extra components"
    );
    let (damage, extra) = crate::item_data::decode_components(Some(&Tag::Compound(
        [(
            "!minecraft:use_effects".into(),
            Tag::Compound(Default::default()),
        )]
        .into_iter()
        .collect(),
    )));
    let removed = ItemStack {
        damage,
        extra,
        ..ItemStack::new("minecraft:stick", 1)
    };
    assert!(matches("removed", &removed));
    assert!(!matches("defaults", &removed));
    assert!(!matches("defaults", &ItemStack::new("minecraft:stick", 0)));
    for speed in [
        Tag::Float((-0.0_f32).to_bits()),
        Tag::Double((-0.0_f64).to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
    ] {
        let tag = Tag::Compound([("speed_multiplier".into(), speed)].into_iter().collect());
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:use_effects",
                &tag,
                &items.component_registry,
            )
            .is_err()
        );
        let mut invalid_stack = ItemStack::new("minecraft:stick", 1);
        invalid_stack
            .data_mut()
            .set_loot_component("minecraft:use_effects", &Some(tag));
        assert!(!matches("zero", &invalid_stack));
    }
    // Unknown Java string keys are ignored, not lossy-decoded into known fields.
    let tag = Tag::Compound(
        [(horizon_core::nbt::JavaString(vec![0xd800]), Tag::Int(1))]
            .into_iter()
            .collect(),
    );
    stack
        .data_mut()
        .set_loot_component("minecraft:use_effects", &Some(tag));
    assert!(matches("defaults", &stack));
    for speed in [
        Tag::Byte(0),
        Tag::Short(0),
        Tag::Int(0),
        Tag::Long(0),
        Tag::Float(0.0_f32.to_bits()),
        Tag::Double(0.0_f64.to_bits()),
    ] {
        let tag = Tag::Compound([("speed_multiplier".into(), speed)].into_iter().collect());
        stack
            .data_mut()
            .set_loot_component("minecraft:use_effects", &Some(tag));
        assert!(matches("zero", &stack));
    }
    let rounded = Tag::Compound(
        [
            ("can_sprint".into(), Tag::Float(f32::NAN.to_bits())),
            (
                "interact_vibrations".into(),
                Tag::Double((-0.0_f64).to_bits()),
            ),
            (
                "speed_multiplier".into(),
                Tag::Double(1.0000000001_f64.to_bits()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    stack
        .data_mut()
        .set_loot_component("minecraft:use_effects", &Some(rounded));
    assert!(
        matches("one", &stack),
        "bounds apply after float conversion"
    );
}
