use super::*;

fn axolotl_variant_recipes(
    items: &Items,
    patch: serde_json::Value,
    strict: bool,
) -> crate::crafting::Recipes {
    let ingredient = serde_json::json!({
        "type": "neoforge:components",
        "items": "minecraft:axolotl_spawn_egg",
        "components": patch,
        "strict": strict
    });
    let root = tempfile::tempdir().unwrap();
    crate::crafting::Recipes::from_sources(
        root.path(),
        items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![
            (
                "fixture:axolotl_shapeless".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient.clone()],
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:axolotl_shaped".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["A"],
                    "key": {"A": ingredient.clone()},
                    "result": {"id": "minecraft:diamond"}
                }),
            ),
            (
                "fixture:axolotl_smelting".into(),
                serde_json::json!({
                    "type": "minecraft:smelting",
                    "ingredient": ingredient,
                    "result": {"id": "minecraft:diamond"},
                    "experience": 0.0,
                    "cookingtime": 20
                }),
            ),
        ],
        &load(),
    )
    .unwrap()
}

#[test]
fn axolotl_variant_recipe_codec_matches_exact_names_without_candidate_allocations() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for (name, different) in [
        ("lucy", "wild"),
        ("wild", "gold"),
        ("gold", "cyan"),
        ("cyan", "blue"),
        ("blue", "lucy"),
    ] {
        // An unqualified component key normalizes, but the enum payload does not.
        let recipes =
            axolotl_variant_recipes(&items, serde_json::json!({"axolotl/variant": name}), false);
        assert_eq!(recipes.crafting.len(), 2);
        assert_eq!(recipes.cooking.len(), 1);
        for (_, recipe) in &recipes.crafting {
            let ingredient = match recipe {
                crate::crafting::Recipe::Shapeless { ingredients, .. } => &ingredients[0],
                crate::crafting::Recipe::Shaped { cells, .. } => cells[0].as_ref().unwrap(),
                _ => panic!("literal crafting recipe expected"),
            };
            assert_eq!(
                ingredient.shown.as_ref().unwrap()[0]
                    .data()
                    .unwrap()
                    .other("minecraft:axolotl/variant"),
                Some(&Tag::String(name.into()))
            );
            assert!(
                !ingredient.test_in(&context, &ItemStack::new("minecraft:axolotl_spawn_egg", 1))
            );
            let mut stack = ItemStack::new("minecraft:axolotl_spawn_egg", 7);
            stack
                .data_mut()
                .set_other("minecraft:axolotl/variant", Some(Tag::String(name.into())));
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..100 {
                    assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
            for value in [
                Tag::String(different.into()),
                Tag::End,
                Tag::Byte(0),
                Tag::Byte(1),
                Tag::List(vec![]),
                Tag::Compound(Default::default()),
                Tag::Int(if name == "lucy" { 0 } else { 1 }),
                Tag::String(horizon_core::nbt::JavaString(vec![114, 101, 100, 0xd800])),
            ] {
                stack
                    .data_mut()
                    .set_other("minecraft:axolotl/variant", Some(value));
                assert!(!ingredient.test_in(&context, &stack));
            }
            stack
                .data_mut()
                .set_other("minecraft:axolotl/variant", Some(Tag::String(name.into())));
            stack.count = 0;
            assert!(!ingredient.test_in(&context, &stack));
        }
        let mut egg = ItemStack::new("minecraft:axolotl_spawn_egg", 1);
        egg.data_mut()
            .set_other("minecraft:axolotl/variant", Some(Tag::String(name.into())));
        assert_eq!(
            recipes
                .craft(&context, &[Some(egg.clone())], 1)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_some());
        egg.data_mut().set_other(
            "minecraft:axolotl/variant",
            Some(Tag::String(different.into())),
        );
        assert!(recipes.craft(&context, &[Some(egg.clone())], 1).is_none());
        assert!(recipes.cook(&context, "minecraft:smelting", &egg).is_none());
    }
}

#[test]
fn axolotl_variant_recipe_defaults_removal_and_strictness_preserve_presence() {
    // Synthetic pack prototype: vanilla eggs and buckets have no variant component.
    let mut items = Items::empty();
    std::sync::Arc::make_mut(&mut items.component_registry)
        .by_registry
        .entry("minecraft:data_component_type".into())
        .or_default()
        .ids
        .push("minecraft:custom_data".into());
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:axolotl_spawn_egg".into(),
        match Tag::compound([("minecraft:axolotl/variant", Tag::String("lucy".into()))]) {
            Tag::Compound(values) => values,
            _ => unreachable!(),
        },
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let exact = axolotl_variant_recipes(
        &items,
        serde_json::json!({"minecraft:axolotl/variant": "lucy"}),
        true,
    );
    let loose = axolotl_variant_recipes(
        &items,
        serde_json::json!({"minecraft:axolotl/variant": "lucy"}),
        false,
    );
    let omitted = axolotl_variant_recipes(&items, serde_json::json!({}), true);
    // Removal payloads require registration, not a successful variant decode.
    let removal = axolotl_variant_recipes(
        &items,
        serde_json::json!({"!minecraft:axolotl/variant": 123}),
        true,
    );
    assert_eq!(removal.crafting.len(), 2);
    let mut egg = ItemStack::new("minecraft:axolotl_spawn_egg", 1);
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(omitted.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(3)));
    assert!(loose.craft(&context, &[Some(egg.clone())], 1).is_some());
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    egg.data_mut().set_other("minecraft:custom_data", None);
    for (value, matches_default) in [("lucy", true), ("wild", false)] {
        egg.data_mut().set_loot_component(
            "minecraft:axolotl/variant",
            &Some(Tag::String(value.into())),
        );
        assert_eq!(
            exact.craft(&context, &[Some(egg.clone())], 1).is_some(),
            matches_default
        );
        assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_none());
    }
    egg.data_mut()
        .set_loot_component("minecraft:axolotl/variant", &None);
    egg.data_mut().set_loot_component(
        "!minecraft:axolotl/variant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    let removed = Tag::compound([(
        "!minecraft:axolotl/variant",
        Tag::compound([] as [(&str, Tag); 0]),
    )]);
    (_, egg.extra) = crate::item_data::decode_components(Some(&removed));
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg.clone())], 1).is_some());
    egg.data_mut().set_loot_component(
        "minecraft:axolotl/variant",
        &Some(Tag::String("wild".into())),
    );
    assert!(exact.craft(&context, &[Some(egg.clone())], 1).is_none());
    assert!(removal.craft(&context, &[Some(egg)], 1).is_none());
    let absent = axolotl_variant_recipes(
        &Items::empty(),
        serde_json::json!({"!minecraft:axolotl/variant": {}}),
        true,
    );
    assert!(
        absent
            .craft(
                &context,
                &[Some(ItemStack::new("minecraft:axolotl_spawn_egg", 1))],
                1
            )
            .is_some()
    );
}

#[test]
fn axolotl_variant_invalid_codec_payloads_omit_whole_recipes() {
    let items = Items::empty();
    for (payload, diagnostic) in [
        (serde_json::json!(0), "Not a string"),
        (serde_json::json!(1), "Not a string"),
        (serde_json::json!(1.0), "Not a string"),
        (serde_json::json!(false), "Not a string"),
        (serde_json::json!({}), "Not a string"),
        (serde_json::json!([]), "Not a string"),
        (serde_json::json!("LUCY"), "Unknown element name:LUCY"),
        (serde_json::json!("arctic"), "Unknown element name:arctic"),
        (
            serde_json::json!("minecraft:lucy"),
            "Unknown element name:minecraft:lucy",
        ),
        (serde_json::json!(""), "Unknown element name:"),
    ] {
        let patch = serde_json::json!({"minecraft:axolotl/variant": payload});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert_eq!(
            error,
            format!("component minecraft:axolotl/variant: {diagnostic}")
        );
        let ingredient = serde_json::json!({
            "type": "neoforge:components",
            "items": "minecraft:axolotl_spawn_egg",
            "components": patch.clone()
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = axolotl_variant_recipes(&items, patch, false);
        assert!(recipes.crafting.is_empty());
        assert!(recipes.cooking.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn axolotl_variant_boundary_refusals_keep_a_valid_sibling() {
    const ID: &str = "minecraft:axolotl/variant";
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
        serde_json::json!("minecraft:lucy"),
        serde_json::json!("LUCY"),
    ] {
        let ingredient = serde_json::json!({"type":"neoforge:components", "items":"minecraft:axolotl_spawn_egg", "components":{(ID):payload}});
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
    let whole_null = serde_json::json!({"type":"neoforge:components", "items":"minecraft:axolotl_spawn_egg", "components":null});
    assert!(Ingredient::parse(&whole_null, &items).is_none());
}
