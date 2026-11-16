use super::*;
#[test]
fn container_loot_component_recipe_decodes_validates_and_matches() {
    use serde_json::json;
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            json!({"minecraft:container_loot":{"loot_table":"fixture:unregistered"}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let ingredient_json = |payload: serde_json::Value, strict| {
        json!({"type":"neoforge:components","items":"minecraft:stick",
            "components":{"minecraft:container_loot":payload},"strict":strict})
    };
    let recipe_json = |ingredient: serde_json::Value| {
        json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],
            "result":{"id":"minecraft:diamond"}})
    };
    let recipes = |ingredient: serde_json::Value| {
        crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:loot".into(), recipe_json(ingredient)),
                (
                    "fixture:control".into(),
                    recipe_json(json!("minecraft:stone")),
                ),
            ],
            &load,
        )
        .unwrap()
    };
    for (payload, seed, table) in [
        (
            json!({"loot_table":"fixture:unregistered"}),
            0_i64,
            "fixture:unregistered",
        ),
        (
            json!({"loot_table":"unregistered","seed":0,"ignored":[1]}),
            0,
            "minecraft:unregistered",
        ),
        (
            json!({"loot_table":":unregistered","seed":-2.75}),
            -2,
            "minecraft:unregistered",
        ),
        (
            json!({"loot_table":"fixture:unregistered","seed":i64::MAX}),
            i64::MAX,
            "fixture:unregistered",
        ),
        (
            json!({"loot_table":"fixture:unregistered","seed":i64::MIN}),
            i64::MIN,
            "fixture:unregistered",
        ),
        (json!({"loot_table":""}), 0, "minecraft:"),
    ] {
        for strict in [false, true] {
            let source = ingredient_json(payload.clone(), strict);
            let ingredient = Ingredient::parse(&source, &items).unwrap();
            let loaded = recipes(source);
            let shown = ingredient.shown.as_ref().unwrap()[0].data().unwrap();
            assert_eq!(
                shown.other("minecraft:container_loot"),
                Some(&Tag::compound([
                    ("loot_table", Tag::String(table.into())),
                    ("seed", Tag::Long(seed)),
                ]))
            );
            let mut stack = ItemStack::new("minecraft:stick", 1);
            stack.data_mut().set_loot_component(
                "minecraft:container_loot",
                &Some(horizon_core::json_nbt::json_to_tag(&payload).unwrap()),
            );
            assert!(loaded.recipe_matches(&context, "fixture:loot", &[Some(stack.clone())], 1));
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                assert!(ingredient.test_in(&context, &stack));
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            stack.data_mut().set_loot_component(
                "minecraft:container_loot",
                &Some(Tag::compound([
                    ("loot_table", Tag::String(table.into())),
                    ("seed", Tag::Long(seed.wrapping_add(1))),
                ])),
            );
            assert!(!loaded.recipe_matches(&context, "fixture:loot", &[Some(stack.clone())], 1));
            stack.data_mut().set_loot_component(
                "minecraft:container_loot",
                &Some(Tag::compound([(
                    "loot_table",
                    Tag::String("fixture:other".into()),
                )])),
            );
            assert!(!ingredient.test_in(&context, &stack));
        }
    }
    let source = ingredient_json(json!({"loot_table":"fixture:unregistered","seed":0}), false);
    let ingredient = Ingredient::parse(&source, &items).unwrap();
    let loaded = recipes(source);
    let mut stack = ItemStack::new("minecraft:stick", 1);
    assert!(loaded.recipe_matches(&context, "fixture:loot", &[Some(stack.clone())], 1));
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_data", &Some(Tag::Int(1)));
    assert!(ingredient.test_in(&context, &stack));
    let strict = recipes(ingredient_json(
        json!({"loot_table":"fixture:unregistered"}),
        true,
    ));
    assert!(!strict.recipe_matches(&context, "fixture:loot", &[Some(stack.clone())], 1));
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_data", &None);
    stack
        .data_mut()
        .set_loot_component("minecraft:container_loot", &None);
    stack.data_mut().set_loot_component(
        "!minecraft:container_loot",
        &Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!loaded.recipe_matches(&context, "fixture:loot", &[Some(stack.clone())], 1));
    let removed = recipes(
        json!({"type":"neoforge:components","items":"minecraft:stick",
        "components":{"!minecraft:container_loot":{}}}),
    );
    assert!(removed.recipe_matches(&context, "fixture:loot", &[Some(stack)], 1));
    assert!(!removed.recipe_matches(
        &context,
        "fixture:loot",
        &[Some(ItemStack::new("minecraft:stick", 1))],
        1
    ));

    for payload in [
        json!({}),
        json!("fixture:unregistered"),
        json!([]),
        json!({"loot_table":1}),
        json!({"loot_table":{"type":"minecraft:empty"}}),
        json!({"loot_table":"Bad:table"}),
        json!({"loot_table":"..:table"}),
        json!({"loot_table":"a:b:c"}),
        json!({"loot_table":"bad path"}),
        json!({"loot_table":"fixture:unregistered","seed":"0"}),
        json!({"loot_table":"fixture:unregistered","seed":{}}),
    ] {
        let source = ingredient_json(payload, false);
        assert!(Ingredient::parse(&source, &items).is_none());
        match crate::json_nbt::json_to_compound(source["components"].as_object().unwrap()) {
            Ok(patch) => {
                let rule = IngredientRule::Components {
                    patch,
                    removed: Vec::new(),
                    strict: false,
                };
                let error = components::validate(&rule, &items).unwrap_err();
                assert!(
                    error.contains("component minecraft:container_loot:"),
                    "{error}"
                );
            }
            // Nested null is refused before a component payload reaches NbtOps.
            Err(error) => assert_eq!(error, horizon_core::json_nbt::JsonNbtError::NestedNull),
        }
        let loaded = recipes(source);
        assert!(loaded.recipe("fixture:loot").is_none());
        assert!(!loaded.recipe_matches(
            &context,
            "fixture:loot",
            &[Some(ItemStack::new("minecraft:stick", 1))],
            1
        ));
        assert!(loaded.recipe_matches(
            &context,
            "fixture:control",
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
    }
    for (raw, expected) in [
        (Tag::Byte(-1), -1),
        (Tag::Short(2), 2),
        (Tag::Int(3), 3),
        (Tag::Long(i64::MIN), i64::MIN),
        (Tag::Float((-0.0_f32).to_bits()), 0),
        (Tag::Double(f64::NAN.to_bits()), 0),
        (Tag::Double(f64::INFINITY.to_bits()), i64::MAX),
        (Tag::Double(f64::NEG_INFINITY.to_bits()), i64::MIN),
        (Tag::Double((-2.75_f64).to_bits()), -2),
    ] {
        let value = Tag::compound([
            ("loot_table", Tag::String(":unregistered".into())),
            ("seed", raw),
        ]);
        let decoded = horizon_command::item::decoded_component_value(
            "minecraft:container_loot",
            &value,
            &items.component_registry,
        )
        .unwrap();
        assert_eq!(decoded.get("seed"), Some(&Tag::Long(expected)));
    }
    for units in [vec![0xd800], vec![0xdc00], vec![0xd83d, 0xde00]] {
        let value = Tag::compound([(
            "loot_table",
            Tag::String(horizon_core::nbt::JavaString(units)),
        )]);
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:container_loot",
                &value,
                &items.component_registry,
            )
            .is_err()
        );
    }
}

#[test]
fn lodestone_tracker_component_recipes_decode_validate_and_match() {
    const ID: &str = "minecraft:lodestone_tracker";
    fn ingredient(patch: Value, strict: bool) -> Value {
        serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:compass",
            "components":patch, "strict":strict
        })
    }
    fn recipe(ingredient: Value) -> Value {
        serde_json::json!({
            "type":"minecraft:smelting", "ingredient":ingredient,
            "result":{"id":"minecraft:diamond"}, "experience":0.0, "cookingtime":20
        })
    }
    fn book(items: &Items, sources: Vec<(&str, Value)>) -> crate::crafting::Recipes {
        let root = tempfile::tempdir().unwrap();
        crate::crafting::Recipes::from_sources(
            root.path(),
            items,
            &horizon_world::loot::PackOverlay::NONE,
            sources
                .into_iter()
                .map(|(id, value)| (id.into(), value))
                .collect(),
            &load(),
        )
        .unwrap()
    }
    fn stack(value: Tag) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:compass", 1);
        stack.data_mut().set_loot_component(ID, &Some(value));
        stack
    }
    fn target(dimension: Tag, pos: Tag) -> Tag {
        Tag::compound([(
            "target",
            Tag::compound([("dimension", dimension), ("pos", pos)]),
        )])
    }
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:compass".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({
                "minecraft:lodestone_tracker":{
                    "target":{"dimension":":unregistered/dimension","pos":[1,-2,3]},
                    "tracked":-2
                },
                "minecraft:max_stack_size":64
            })
            .as_object()
            .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let check =
        |recipes: &crate::crafting::Recipes, candidate: &ItemStack, expected: Option<&str>| {
            // Build the cooking index outside the candidate-allocation measurement.
            let _ = recipes.cook(&context, "minecraft:smelting", candidate);
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                let cooked = recipes.cook(&context, "minecraft:smelting", candidate);
                assert_eq!(cooked.map(|recipe| recipe.id.as_str()), expected);
                if let Some(cooked) = cooked {
                    assert_eq!(cooked.result.item.as_ref(), "minecraft:diamond");
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
        };
    let literal: Value = serde_json::from_str(
        r#"{"target":{"dimension":"unregistered/dimension","pos":[1.9,-2.9,4294967299],"ignored":false},"ignored":{"future":true}}"#,
    )
    .unwrap();
    let patch = serde_json::json!({ID:literal});
    let recipes = book(
        &items,
        vec![("fixture:target", recipe(ingredient(patch.clone(), false)))],
    );
    let shown = Ingredient::parse(&ingredient(patch.clone(), false), &items).unwrap();
    assert_eq!(
        shown.shown.as_ref().unwrap()[0].data().unwrap().other(ID),
        Some(&target(
            Tag::String("minecraft:unregistered/dimension".into()),
            Tag::IntArray(vec![1, -2, 3]),
        )),
    );
    let plain = ItemStack::new("minecraft:compass", 1);
    check(&recipes, &plain, Some("fixture:target"));
    for dimension in [
        "unregistered/dimension",
        ":unregistered/dimension",
        "minecraft:unregistered/dimension",
    ] {
        for pos in [
            Tag::IntArray(vec![1, -2, 3]),
            Tag::ByteArray(vec![1, 254, 3]),
            Tag::LongArray(vec![4294967297, -2, 3]),
            Tag::List(vec![
                Tag::Short(1),
                Tag::Float((-2.9f32).to_bits()),
                Tag::Long(4294967299),
            ]),
        ] {
            check(
                &recipes,
                &stack(target(Tag::String(dimension.into()), pos)),
                Some("fixture:target"),
            );
        }
    }
    for payload in [
        r#"{"target":{"dimension":"fixture:unregistered/dimension","pos":[1,-2,3]}}"#,
        r#"{"target":{"dimension":"Minecraft:unregistered/dimension","pos":[1,-2,3]}}"#,
        r#"{"target":{"dimension":"unregistered/dimension","pos":[1,-2,4]}}"#,
        r#"{"target":{"dimension":"unregistered/dimension","pos":[1,-2]}}"#,
        r#"{"target":{"dimension":"unregistered/dimension","pos":[1,-2,3]},"tracked":false}"#,
        r#"{"target":{"dimension":"unregistered/dimension","pos":[1,-2,3]},"tracked":"true"}"#,
        r#"{}"#,
    ] {
        let value: Value = serde_json::from_str(payload).unwrap();
        check(
            &recipes,
            &stack(crate::json_nbt::json_to_tag(&value).unwrap()),
            None,
        );
    }
    let mut empty = plain.clone();
    empty.count = 0;
    check(&recipes, &empty, None);
    check(&recipes, &ItemStack::new("minecraft:stone", 1), None);

    // Strictness counts prototype components, but not unknown record fields.
    let strict = book(
        &items,
        vec![("fixture:strict", recipe(ingredient(patch.clone(), true)))],
    );
    check(&strict, &plain, None);
    let mut complete_patch = patch.clone();
    complete_patch["minecraft:max_stack_size"] = serde_json::json!(64);
    let complete = book(
        &items,
        vec![("fixture:complete", recipe(ingredient(complete_patch, true)))],
    );
    check(&complete, &plain, Some("fixture:complete"));
    let mut extra = plain.clone();
    extra.data_mut().set_loot_component(
        "minecraft:custom_data",
        &Some(Tag::compound([("opaque", Tag::Int(7))])),
    );
    check(&complete, &extra, None);
    check(&recipes, &extra, Some("fixture:target"));
    let mut without_max = plain.clone();
    (_, without_max.extra) = crate::item_data::decode_components(Some(&Tag::compound([(
        "!minecraft:max_stack_size",
        Tag::Compound(Default::default()),
    )])));
    check(&strict, &without_max, Some("fixture:strict"));

    let removal = book(
        &items,
        vec![(
            "fixture:removed",
            recipe(ingredient(
                serde_json::json!({"!minecraft:lodestone_tracker":{}}),
                false,
            )),
        )],
    );
    check(&removal, &plain, None);
    let mut removed = plain.clone();
    (_, removed.extra) = crate::item_data::decode_components(Some(&Tag::compound([(
        "!minecraft:lodestone_tracker",
        Tag::Compound(Default::default()),
    )])));
    check(&removal, &removed, Some("fixture:removed"));
    check(&recipes, &removed, None);
    let mut reset = stack(Tag::Compound(Default::default()));
    reset.data_mut().set_loot_component(ID, &None);
    check(&recipes, &reset, Some("fixture:target"));
    check(&removal, &reset, None);

    // Optional defaults are not defaults for malformed, present fields.
    for (literal, tracked) in [
        (r#"{}"#, true),
        (r#"{"tracked":true,"ignored":5}"#, true),
        (r#"{"tracked":-0.25}"#, true),
        (r#"{"tracked":false}"#, false),
        (r#"{"tracked":0}"#, false),
    ] {
        let value: Value = serde_json::from_str(literal).unwrap();
        let recipes = book(
            &items,
            vec![(
                "fixture:targetless",
                recipe(ingredient(serde_json::json!({ID:value}), false)),
            )],
        );
        let matching = stack(Tag::compound([("tracked", Tag::Byte(i8::from(tracked)))]));
        check(&recipes, &matching, Some("fixture:targetless"));
        check(
            &recipes,
            &stack(Tag::compound([("tracked", Tag::Byte(i8::from(!tracked)))])),
            None,
        );
        check(&recipes, &plain, None);
        if !tracked {
            for zero in [
                Tag::Float((-0.0f32).to_bits()),
                Tag::Double((-0.0f64).to_bits()),
            ] {
                check(
                    &recipes,
                    &stack(Tag::compound([("tracked", zero)])),
                    Some("fixture:targetless"),
                );
            }
        } else {
            check(
                &recipes,
                &stack(Tag::Compound(Default::default())),
                Some("fixture:targetless"),
            );
            check(
                &recipes,
                &stack(Tag::compound([(
                    "tracked",
                    Tag::Double(f64::NAN.to_bits()),
                )])),
                Some("fixture:targetless"),
            );
        }
    }
    // ResourceKey.codec accepts pack-defined keys, even empty identifier paths.
    for dimension in ["fixture:pack_only", "", ":", "minecraft:"] {
        let recipes = book(
            &items,
            vec![(
                "fixture:dimension",
                recipe(ingredient(
                    serde_json::json!({ID:{
                        "target":{"dimension":dimension,"pos":[2147483647,-2147483648,0]}
                    }}),
                    false,
                )),
            )],
        );
        let candidate = stack(target(
            Tag::String(dimension.into()),
            Tag::List(vec![
                Tag::Float(f32::INFINITY.to_bits()),
                Tag::Double(f64::NEG_INFINITY.to_bits()),
                Tag::Float(f32::NAN.to_bits()),
            ]),
        ));
        check(&recipes, &candidate, Some("fixture:dimension"));
        check(&recipes, &plain, None);
    }
    for units in [vec![0xd800], vec![0xdc00], vec![0xd83d, 0xde00]] {
        let invalid = target(
            Tag::String(horizon_core::nbt::JavaString(units)),
            Tag::IntArray(vec![1, -2, 3]),
        );
        assert!(
            horizon_command::item::decoded_component_value(ID, &invalid, &items.component_registry)
                .is_err()
        );
        assert!(
            horizon_command::item::decode_component(ID, &invalid, &items.component_registry)
                .is_err()
        );
        check(&recipes, &stack(invalid), None);
    }
    for literal in [
        r#"5"#,
        r#"true"#,
        r#""tracker""#,
        r#"[]"#,
        r#"{"target":5}"#,
        r#"{"target":{}}"#,
        r#"{"target":{"dimension":"overworld"}}"#,
        r#"{"target":{"pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":5,"pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"Minecraft:overworld","pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"minecraft:Overworld","pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"bad namespace:path","pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"..:path","pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"a:b:c","pos":[1,2,3]}}"#,
        r#"{"target":{"dimension":"overworld","pos":[]}}"#,
        r#"{"target":{"dimension":"overworld","pos":[1,2]}}"#,
        r#"{"target":{"dimension":"overworld","pos":[1,2,3,4]}}"#,
        r#"{"target":{"dimension":"overworld","pos":[1,"2",3]}}"#,
        r#"{"target":{"dimension":"overworld","pos":5}}"#,
        r#"{"tracked":"true"}"#,
        r#"{"tracked":{}}"#,
        r#"{"tracked":[]}"#,
    ] {
        let value: Value = serde_json::from_str(literal).unwrap();
        let invalid = ingredient(serde_json::json!({ID:value}), false);
        assert!(Ingredient::parse(&invalid, &items).is_none(), "{literal}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(invalid["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:lodestone_tracker:"),
            "{error}"
        );
        let loaded = book(
            &items,
            vec![
                ("fixture:refused", recipe(invalid)),
                (
                    "fixture:kept",
                    recipe(ingredient(serde_json::json!({ID:{}}), false)),
                ),
            ],
        );
        assert!(
            loaded
                .cooking
                .iter()
                .all(|recipe| recipe.id != "fixture:refused")
        );
        check(
            &loaded,
            &stack(Tag::Compound(Default::default())),
            Some("fixture:kept"),
        );
        check(&loaded, &plain, None);
    }
    // Raw NBT can contain End; JSON deliberately refuses nested null first.
    let invalid = Tag::End;
    assert!(
        horizon_command::item::decoded_component_value(ID, &invalid, &items.component_registry)
            .is_err()
    );
    check(&recipes, &stack(invalid), None);
}

#[test]
fn tooltip_display_recipe_uses_registered_component_sets_and_numeric_booleans() {
    use crate::crafting::{Recipe, Recipes};
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    use std::sync::Arc;

    let mut items = Items::empty();
    let registry = Arc::make_mut(&mut items.component_registry);
    registry
        .by_registry
        .entry("minecraft:data_component_type".into())
        .or_default()
        .ids = vec![
        "minecraft:tooltip_display".into(),
        "minecraft:damage".into(),
        "minecraft:unbreakable".into(),
        "minecraft:banner_patterns".into(),
        "fixture:transient".into(),
    ];
    // A declared persistent mod component need not have a built-in payload decoder.
    registry
        .persistent_components
        .push("fixture:declared".into());
    // Synthetic test prototype, not a claim about vanilla stone's defaults.
    items.component_defaults = Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        crate::json_nbt::json_to_compound(json!({
            "minecraft:tooltip_display": {
                "hide_tooltip": -0.0,
                "hidden_components": ["fixture:transient", "damage", "fixture:declared", "unbreakable"]
            }
        }).as_object().unwrap()).unwrap(),
    )]));
    let root = tempfile::tempdir().unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let compile = |item: &str, patch: serde_json::Value, strict: bool| {
        Recipes::from_sources(root.path(), &items, &PackOverlay::NONE, vec![(
            "fixture:tooltip".into(),
            json!({
                "type":"minecraft:crafting_shaped", "pattern":["X"],
                "key":{"X":{"type":"neoforge:components","items":item,"components":patch,"strict":strict}},
                "result":{"id":"minecraft:diamond"}
            }),
        )], &load).unwrap()
    };
    let patch = json!({"minecraft:tooltip_display":{
        "hide_tooltip":0,
        "hidden_components":["unbreakable","fixture:declared","damage","fixture:transient","damage"]
    }});
    let recipe = compile("minecraft:stone", patch.clone(), false);
    let strict = compile("minecraft:stone", patch, true);
    let stone = ItemStack::new("minecraft:stone", 1);
    let outcome = |recipes: &Recipes, stack: &ItemStack| {
        recipes
            .craft(&context, &[Some(stack.clone())], 1)
            .map(|stack| stack.item)
    };
    assert_eq!(
        outcome(&recipe, &stone).as_deref(),
        Some("minecraft:diamond")
    );
    assert_eq!(
        outcome(&strict, &stone).as_deref(),
        Some("minecraft:diamond")
    );
    assert!(outcome(&recipe, &ItemStack::new("minecraft:dirt", 1)).is_none());
    let mut explicit = stone.clone();
    explicit.data_mut().set_other(
        "minecraft:tooltip_display",
        Some(Tag::compound([
            ("hide_tooltip", Tag::Double((-0.0_f64).to_bits())),
            (
                "hidden_components",
                Tag::List(vec![
                    Tag::String("fixture:declared".into()),
                    Tag::String("minecraft:damage".into()),
                    Tag::String("minecraft:unbreakable".into()),
                    Tag::String("fixture:transient".into()),
                    Tag::String(":damage".into()),
                ]),
            ),
            ("ignored", Tag::Int(17)),
        ])),
    );
    assert_eq!(
        outcome(&strict, &explicit).as_deref(),
        Some("minecraft:diamond")
    );
    let Recipe::Shaped { cells, .. } = &recipe.crafting[0].1 else {
        panic!("shaped recipe");
    };
    let predicate = cells[0].as_ref().unwrap();
    let (_, counts) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..32 {
            assert!(predicate.test_in(&context, std::hint::black_box(&explicit)));
            assert!(predicate.test_in(&context, std::hint::black_box(&stone)));
        }
    });
    assert_eq!(counts.calls, 0, "{counts:?}");
    assert_eq!(counts.bytes, 0);
    explicit.damage = 3;
    assert!(outcome(&recipe, &explicit).is_some());
    assert!(
        outcome(&strict, &explicit).is_none(),
        "strictness counts extra components"
    );
    explicit.damage = 0;
    for value in [
        Tag::compound([("hide_tooltip", Tag::Byte(1))]),
        Tag::compound([(
            "hidden_components",
            Tag::List(vec![Tag::String("damage".into())]),
        )]),
        Tag::compound([(
            "hidden_components",
            Tag::List(vec![
                Tag::String("damage".into()),
                Tag::String("fixture:unknown".into()),
            ]),
        )]),
    ] {
        explicit
            .data_mut()
            .set_other("minecraft:tooltip_display", Some(value));
        assert!(outcome(&recipe, &explicit).is_none());
    }
    let empty = compile(
        "minecraft:stone",
        json!({"minecraft:tooltip_display":{}}),
        false,
    );
    for collection in [
        Tag::List(vec![]),
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
    ] {
        explicit.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([
                ("hide_tooltip", Tag::Float((-0.0_f32).to_bits())),
                ("hidden_components", collection),
            ])),
        );
        assert_eq!(
            outcome(&empty, &explicit).as_deref(),
            Some("minecraft:diamond")
        );
    }
    let hidden = compile(
        "minecraft:stone",
        json!({"minecraft:tooltip_display":{"hide_tooltip":0.5}}),
        false,
    );
    explicit.data_mut().set_other(
        "minecraft:tooltip_display",
        Some(Tag::compound([("hide_tooltip", Tag::Long(1_i64 << 32))])),
    );
    assert_eq!(
        outcome(&hidden, &explicit).as_deref(),
        Some("minecraft:diamond")
    );
    explicit.data_mut().set_other(
        "minecraft:tooltip_display",
        Some(Tag::compound([(
            "hide_tooltip",
            Tag::Double(f64::NAN.to_bits()),
        )])),
    );
    assert_eq!(
        outcome(&hidden, &explicit).as_deref(),
        Some("minecraft:diamond")
    );
    let removal = compile(
        "minecraft:stone",
        json!({"!minecraft:tooltip_display":{}}),
        true,
    );
    assert!(
        outcome(&removal, &stone).is_none(),
        "the prototype is present"
    );
    let mut removed = stone.clone();
    removed.data_mut().set_other(
        "!minecraft:tooltip_display",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(
        outcome(&recipe, &removed).is_none(),
        "removal wins over the prototype"
    );
    assert_eq!(
        outcome(&removal, &removed).as_deref(),
        Some("minecraft:diamond")
    );
    let banner = compile(
        "minecraft:white_banner",
        json!({"minecraft:tooltip_display":{
            "hidden_components":["banner_patterns","minecraft:banner_patterns"]
        }}),
        false,
    );
    let mut ominous = ItemStack::new("minecraft:white_banner", 1);
    ominous.data_mut().ominous_banner = true;
    assert_eq!(
        outcome(&banner, &ominous).as_deref(),
        Some("minecraft:diamond")
    );

    for invalid in [
        json!(true),
        json!({"hide_tooltip":"true"}),
        json!({"hide_tooltip":[]}),
        json!({"hidden_components":"damage"}),
        json!({"hidden_components":[1]}),
        json!({"hidden_components":["damage","fixture:unknown"]}),
        json!({"hidden_components":["Bad:identifier"]}),
    ] {
        let rejected = compile(
            "minecraft:stone",
            json!({"minecraft:tooltip_display":invalid}),
            false,
        );
        assert!(rejected.crafting.is_empty());
        assert!(rejected.by_type.values().all(|recipes| recipes.is_empty()));
        assert!(outcome(&rejected, &stone).is_none());
    }
}
