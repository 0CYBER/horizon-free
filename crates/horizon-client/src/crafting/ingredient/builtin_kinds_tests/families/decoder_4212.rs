use super::*;
#[test]
fn attack_range_persistent_codec_matches_recipes_without_candidate_allocations() {
    use crate::crafting::{Recipe, Recipes};
    use horizon_world::loot::PackOverlay;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn file(patch: Value, strict: bool, item: &str) -> Value {
        json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"neoforge:components","items":item,
                "components":patch,"strict":strict}],
            "result":{"id":"minecraft:diamond"}
        })
    }
    fn ingredient(recipes: &Recipes) -> &Ingredient {
        let Recipe::Shapeless { ingredients, .. } = recipes.recipe("attack_range:recipe").unwrap()
        else {
            panic!("expected shapeless recipe");
        };
        &ingredients[0]
    }
    fn stack(item: &str, value: Tag) -> ItemStack {
        let mut stack = ItemStack::new(item, 3);
        stack
            .data_mut()
            .set_other("minecraft:attack_range", Some(value));
        stack
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data/minecraft/recipe")).unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut items = Items::empty();
    // Synthetic test prototypes, not vanilla defaults: stone inherits an empty
    // range record and damage; dirt inherits a distinct six-field range.
    items.component_defaults = Arc::new(std::collections::HashMap::from([
        (
            "minecraft:stone".into(),
            horizon_core::nbt::Compound::from([
                (
                    "minecraft:attack_range".into(),
                    Tag::compound([("ignored", Tag::Int(9))]),
                ),
                ("minecraft:damage".into(), Tag::Int(0)),
            ]),
        ),
        (
            "minecraft:dirt".into(),
            horizon_core::nbt::Compound::from([(
                "minecraft:attack_range".into(),
                Tag::compound([
                    ("min_reach", Tag::Byte(1)),
                    ("max_reach", Tag::Short(2)),
                    ("min_creative_reach", Tag::Int(3)),
                    ("max_creative_reach", Tag::Long(4)),
                    ("hitbox_margin", Tag::Float(0.5f32.to_bits())),
                    ("mob_factor", Tag::Double(1.5f64.to_bits())),
                ]),
            )]),
        ),
    ]));
    let compile = |patch, strict, item| {
        Recipes::from_sources(
            root.path(),
            &items,
            &PackOverlay::NONE,
            vec![("attack_range:recipe".into(), file(patch, strict, item))],
            &load,
        )
        .unwrap()
    };
    let matches = |recipes: &Recipes, stack: &ItemStack| {
        recipes.recipe_matches(&context, "attack_range:recipe", &[Some(stack.clone())], 1)
    };
    let defaults = compile(
        json!({"minecraft:attack_range":{}}),
        false,
        "minecraft:stone",
    );
    let stone = ItemStack::new("minecraft:stone", 3);
    assert!(matches(&defaults, &stone));
    assert!(!matches(&defaults, &ItemStack::new("minecraft:dirt", 1)));
    assert!(!matches(&defaults, &ItemStack::new("minecraft:stone", 0)));
    assert_eq!(
        ingredient(&defaults).shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:attack_range"),
        Some(&Tag::compound([
            ("min_reach", Tag::Float(0.0f32.to_bits())),
            ("max_reach", Tag::Float(3.0f32.to_bits())),
            ("min_creative_reach", Tag::Float(0.0f32.to_bits())),
            ("max_creative_reach", Tag::Float(5.0f32.to_bits())),
            ("hitbox_margin", Tag::Float(0.3f32.to_bits())),
            ("mob_factor", Tag::Float(1.0f32.to_bits())),
        ]))
    );
    let full = compile(
        json!({"minecraft:attack_range":{"min_reach":1,"max_reach":2,
            "min_creative_reach":3,"max_creative_reach":4,
            "hitbox_margin":0.5,"mob_factor":1.5,"unknown":{"ignored":true}}}),
        true,
        "minecraft:dirt",
    );
    let dirt = ItemStack::new("minecraft:dirt", 1);
    assert!(matches(&full, &dirt));
    let mut override_dirt = dirt.clone();
    override_dirt.data_mut().set_other(
        "minecraft:attack_range",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(
        !matches(&full, &override_dirt),
        "explicit values replace the prototype"
    );

    // Every field is independent, including min > max: the codec has no
    // cross-field ordering validation. All NBT numeric widths become f32.
    for (name, maximum) in [
        ("min_reach", 64.0f64),
        ("max_reach", 64.0),
        ("min_creative_reach", 64.0),
        ("max_creative_reach", 64.0),
        ("hitbox_margin", 1.0),
        ("mob_factor", 2.0),
    ] {
        for number in [0.0, maximum] {
            let query = compile(
                json!({"minecraft:attack_range":{(name):number}}),
                false,
                "minecraft:stone",
            );
            for numeric in [
                Tag::Byte(number as i8),
                Tag::Short(number as i16),
                Tag::Int(number as i32),
                Tag::Long(number as i64),
                Tag::Float((number as f32).to_bits()),
                Tag::Double(number.to_bits()),
            ] {
                let current = stack(
                    "minecraft:stone",
                    Tag::compound([(name, numeric), ("ignored", Tag::Int(3))]),
                );
                assert!(matches(&query, &current), "{name}={number}");
            }
        }
        for boolean in [false, true] {
            let query = compile(
                json!({"minecraft:attack_range":{(name):boolean}}),
                false,
                "minecraft:stone",
            );
            let current = stack(
                "minecraft:stone",
                Tag::compound([(name, Tag::Int(i32::from(boolean)))]),
            );
            assert!(
                matches(&query, &current),
                "NbtOps byte booleans are numeric"
            );
        }
        for invalid in [
            Tag::Float((-0.0f32).to_bits()),
            Tag::Double((-0.0f64).to_bits()),
            Tag::Float(f32::NAN.to_bits()),
            Tag::Double(f64::NAN.to_bits()),
            Tag::Float(f32::INFINITY.to_bits()),
            Tag::Float(f32::NEG_INFINITY.to_bits()),
            Tag::Long(i64::MAX),
            Tag::Double(f64::INFINITY.to_bits()),
        ] {
            let current = stack("minecraft:stone", Tag::compound([(name, invalid.clone())]));
            assert!(!matches(&defaults, &current), "{name}: {invalid:?}");
            assert!(
                horizon_command::item::decoded_component_value(
                    "minecraft:attack_range",
                    &Tag::compound([(name, invalid)]),
                    &items.component_registry,
                )
                .is_err()
            );
        }
        for invalid in [
            json!(-0.1),
            json!(maximum + 0.1),
            json!("fixture:provider"),
            json!([]),
            json!({}),
        ] {
            let valid = file(
                json!({"minecraft:attack_range":{}}),
                false,
                "minecraft:stone",
            );
            let invalid = file(
                json!({"minecraft:attack_range":{(name):invalid}}),
                false,
                "minecraft:stone",
            );
            let files = [
                (
                    "recipe/attack_range:recipe".into(),
                    vec![
                        serde_json::to_vec(&valid).unwrap(),
                        serde_json::to_vec(&invalid).unwrap(),
                    ],
                ),
                (
                    "recipe/fixture:sibling".into(),
                    vec![serde_json::to_vec(&file(json!({}), false, "minecraft:stick")).unwrap()],
                ),
            ];
            let omitted =
                Recipes::load(root.path(), &items, &PackOverlay::new(&files), &load).unwrap();
            assert_eq!(omitted.crafting.len(), 1, "{name}: {invalid}");
            assert!(
                omitted
                    .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                    .is_some()
            );
            assert!(omitted.recipe("attack_range:recipe").is_none());
        }
    }
    for invalid in [json!(1), json!("fixture:provider"), json!([]), Value::Null] {
        super::decoder_contracts::assert_malformed_decoder_reload_keeps_sibling(
            &items,
            json!({"minecraft:attack_range":invalid}),
        );
    }
    let crossed = compile(
        json!({"minecraft:attack_range":{"min_reach":64,"max_reach":0,
            "min_creative_reach":64,"max_creative_reach":0}}),
        false,
        "minecraft:stone",
    );
    assert!(matches(
        &crossed,
        &stack(
            "minecraft:stone",
            Tag::compound([
                ("min_reach", Tag::Int(64)),
                ("max_reach", Tag::Int(0)),
                ("min_creative_reach", Tag::Int(64)),
                ("max_creative_reach", Tag::Int(0)),
            ])
        )
    ));
    // JSON's integral negative zero is intentionally converted to NBT +0.
    let json_zero = compile(
        json!({"minecraft:attack_range":{"min_reach":-0.0}}),
        false,
        "minecraft:stone",
    );
    assert!(matches(&json_zero, &stone));
    let rounded = stack(
        "minecraft:stone",
        Tag::compound([("max_reach", Tag::Double(3.00000001f64.to_bits()))]),
    );
    assert!(
        matches(&defaults, &rounded),
        "comparison uses boxed f32, not f64 or raw NBT"
    );
    let strict = compile(
        json!({"minecraft:attack_range":{}}),
        true,
        "minecraft:stone",
    );
    assert!(
        !matches(&strict, &stone),
        "strict components include inherited damage"
    );
    let strict_all = compile(
        json!({"minecraft:attack_range":{},"minecraft:damage":0}),
        true,
        "minecraft:stone",
    );
    assert!(matches(&strict_all, &stone));
    let strict_remove = compile(
        json!({"minecraft:attack_range":{},"!minecraft:damage":{}}),
        true,
        "minecraft:stone",
    );
    let mut removed_damage = stone.clone();
    removed_damage.data_mut().set_other(
        "!minecraft:damage",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(matches(&strict_remove, &removed_damage));
    let mut removed_range = stone.clone();
    removed_range.data_mut().set_other(
        "!minecraft:attack_range",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!matches(&defaults, &removed_range));
    let removal = compile(
        json!({"!minecraft:attack_range":{"min_reach":"not decoded"}}),
        false,
        "minecraft:stone",
    );
    assert!(!matches(&removal, &stone));
    assert!(matches(&removal, &removed_range));
    let type_only = compile(json!({}), false, "minecraft:stone");
    assert!(matches(&type_only, &stone));
    assert!(matches(&type_only, &removed_range));
    let changed = stack(
        "minecraft:stone",
        Tag::compound([("mob_factor", Tag::Double(1.5f64.to_bits()))]),
    );
    assert!(!matches(&defaults, &changed));
    assert!(matches(&type_only, &changed));
    let candidate = ingredient(&defaults);
    let full_candidate = ingredient(&full);
    let (_, counts) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(candidate.test_in(&context, &stone)));
            assert!(std::hint::black_box(candidate.test_in(&context, &rounded)));
            assert!(!std::hint::black_box(candidate.test_in(&context, &changed)));
            assert!(std::hint::black_box(
                full_candidate.test_in(&context, &dirt)
            ));
        }
    });
    assert_eq!(counts.calls, 0, "{counts:?}");
    assert_eq!(counts.bytes, 0, "{counts:?}");
}
