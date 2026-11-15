use super::*;
#[test]
fn damage_resistant_component_recipes_use_loaded_holder_sets() {
    use horizon_world::damage_source::Catalog;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;

    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("items.json"),
        r##"{"minecraft:stick":{"minecraft:damage_resistant":{"types":"#fixture:hot"}}}"##,
    )
    .unwrap();
    std::fs::write(
        root.path().join("damage-catalog.json"),
        json!({"data_version":horizon_world::DATA_VERSION,"types":[]}).to_string(),
    )
    .unwrap();
    let definition = json!({"message_id":"heat","scaling":"never","exhaustion":0});
    let files = vec![
        (
            "damage_type/fixture:heat".into(),
            vec![definition.to_string().into_bytes()],
        ),
        (
            "damage_type/minecraft:ember".into(),
            vec![definition.to_string().into_bytes()],
        ),
        (
            "tags/fixture:damage_type/hot".into(),
            vec![br#"{"values":["fixture:heat"]}"#.to_vec()],
        ),
        (
            "tags/fixture:damage_type/same_members".into(),
            vec![br#"{"values":["fixture:heat"]}"#.to_vec()],
        ),
        (
            "tags/minecraft:damage_type/is_fire".into(),
            vec![br#"{"values":["fixture:heat"]}"#.to_vec()],
        ),
    ];
    let overlay = PackOverlay::new(&files);
    let defaults = horizon_world::item_defaults::read(root.path()).unwrap();
    let mut items = Items::catalog(root.path(), &overlay, &defaults).unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let recipes = |patch: Value, strict: bool, items: &Items| {
        crate::crafting::Recipes::from_sources(
            root.path(),
            items,
            &overlay,
            vec![
                ("fixture:resistant".into(), json!({
                    "type":"minecraft:smelting",
                    "ingredient":{"type":"neoforge:components","items":"minecraft:stick","components":patch,"strict":strict},
                    "result":{"id":"minecraft:diamond"}
                })),
                ("fixture:sibling".into(), json!({
                    "type":"minecraft:smelting","ingredient":"minecraft:stone",
                    "result":{"id":"minecraft:gold_ingot"}
                })),
            ],
            &load,
        )
        .unwrap()
    };
    let set = |stack: &mut ItemStack, value: Tag| {
        stack
            .data_mut()
            .set_loot_component("minecraft:damage_resistant", &Some(value));
    };
    let tag = |value: Value| crate::json_nbt::json_to_tag(&value).unwrap();
    for (literal, matching, different) in [
        (
            json!({"types":"#fixture:hot","ignored":{"field":7}}),
            vec![
                json!({"types":"#fixture:hot"}),
                json!({"types":"#fixture:hot","extra":false}),
            ],
            vec![
                json!({"types":"#fixture:same_members"}),
                json!({"types":["fixture:heat"]}),
            ],
        ),
        (
            json!({"types":"#is_fire"}),
            vec![
                json!({"types":"#is_fire"}),
                json!({"types":"#:is_fire"}),
                json!({"types":"#minecraft:is_fire"}),
            ],
            vec![
                json!({"types":"#fixture:hot"}),
                json!({"types":"fixture:heat"}),
            ],
        ),
        (
            json!({"types":":ember","ignored":5}),
            vec![
                json!({"types":"ember"}),
                json!({"types":"minecraft:ember"}),
                json!({"types":[":ember"]}),
            ],
            vec![
                json!({"types":"fixture:heat"}),
                json!({"types":["ember","ember"]}),
                json!({"types":[]}),
            ],
        ),
        (
            json!({"types":["fixture:heat","ember","fixture:heat"]}),
            vec![json!({"types":["fixture:heat",":ember","fixture:heat"]})],
            vec![
                json!({"types":["ember","fixture:heat","fixture:heat"]}),
                json!({"types":["fixture:heat","ember"]}),
            ],
        ),
        (
            json!({"types":[]}),
            vec![json!({"types":[]})],
            vec![json!({"types":"ember"})],
        ),
    ] {
        let book = recipes(json!({"minecraft:damage_resistant":literal}), false, &items);
        let mut stack = ItemStack::new("minecraft:stick", 1);
        for value in matching {
            set(&mut stack, tag(value));
            let cooked = book.cook(&context, "minecraft:smelting", &stack).unwrap();
            assert_eq!(cooked.id, "fixture:resistant");
            assert_eq!(&*cooked.result.item, "minecraft:diamond");
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                for _ in 0..20 {
                    assert_eq!(
                        book.cook(&context, "minecraft:smelting", &stack)
                            .map(|recipe| recipe.id.as_str()),
                        Some("fixture:resistant"),
                    );
                }
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
            assert_eq!(allocations.bytes, 0);
        }
        for value in different {
            set(&mut stack, tag(value));
            assert!(book.cook(&context, "minecraft:smelting", &stack).is_none());
        }
        assert_eq!(
            book.cook(
                &context,
                "minecraft:smelting",
                &ItemStack::new("minecraft:stone", 1)
            )
            .unwrap()
            .id,
            "fixture:sibling",
        );
    }
    let named = recipes(
        json!({"minecraft:damage_resistant":{"types":"#fixture:hot"}}),
        false,
        &items,
    );
    let strict = recipes(
        json!({"minecraft:damage_resistant":{"types":"#fixture:hot"}}),
        true,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    assert!(named.cook(&context, "minecraft:smelting", &stack).is_some());
    assert!(
        strict
            .cook(&context, "minecraft:smelting", &stack)
            .is_some()
    );
    stack
        .data_mut()
        .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
    assert!(named.cook(&context, "minecraft:smelting", &stack).is_some());
    assert!(
        strict
            .cook(&context, "minecraft:smelting", &stack)
            .is_none()
    );
    let absent = recipes(json!({"!minecraft:damage_resistant":{}}), false, &items);
    assert!(
        absent
            .cook(&context, "minecraft:smelting", &stack)
            .is_none()
    );
    stack
        .data_mut()
        .set_loot_component("minecraft:damage_resistant", &None);
    stack.data_mut().set_loot_component(
        "!minecraft:damage_resistant",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(
        absent
            .cook(&context, "minecraft:smelting", &stack)
            .is_some()
    );
    assert!(named.cook(&context, "minecraft:smelting", &stack).is_none());
    stack.count = 0;
    assert!(
        absent
            .cook(&context, "minecraft:smelting", &stack)
            .is_none()
    );

    for value in [
        json!(5),
        json!([]),
        json!({}),
        json!({"types":5}),
        json!({"types":[5]}),
        json!({"types":{"message_id":"heat","scaling":"never","exhaustion":0}}),
        json!({"types":["#fixture:hot"]}),
        json!({"types":"fixture:missing"}),
        json!({"types":"#fixture:missing"}),
        json!({"types":["fixture:heat","fixture:missing"]}),
        json!({"types":"Minecraft:ember"}),
        json!({"types":"#Fixture:hot"}),
        json!({"types":"..:ember"}),
        json!({"types":"#..:hot"}),
        json!({"types":"a:b:c"}),
    ] {
        let patch = json!({"minecraft:damage_resistant":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:damage_resistant:"),
            "{error}"
        );
        let book = recipes(patch, false, &items);
        assert!(
            book.cook(
                &context,
                "minecraft:smelting",
                &ItemStack::new("minecraft:stick", 1)
            )
            .is_none()
        );
        assert_eq!(
            book.cook(
                &context,
                "minecraft:smelting",
                &ItemStack::new("minecraft:stone", 1)
            )
            .unwrap()
            .id,
            "fixture:sibling",
        );
    }
    let empty = recipes(
        json!({"minecraft:damage_resistant":{"types":[]}}),
        false,
        &items,
    );
    for value in [
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
    ] {
        let value = Tag::compound([("types", value)]);
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:damage_resistant",
                &value,
                &items.component_registry
            )
            .unwrap(),
            Tag::compound([("types", Tag::List(vec![]))]),
        );
        let mut stack = ItemStack::new("minecraft:stick", 1);
        set(&mut stack, value);
        assert!(empty.cook(&context, "minecraft:smelting", &stack).is_some());
    }
    for value in [
        Tag::End,
        Tag::Byte(0),
        Tag::Float((-0.0_f32).to_bits()),
        Tag::Double(f64::NAN.to_bits()),
        Tag::ByteArray(vec![0]),
        Tag::IntArray(vec![0]),
        Tag::LongArray(vec![0]),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        Tag::String(horizon_core::nbt::JavaString(vec![u16::from(b'#'), 0xdc00])),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd83d, 0xde00])),
    ] {
        let value = Tag::compound([("types", value)]);
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:damage_resistant",
                &value,
                &items.component_registry
            )
            .is_err()
        );
        let mut stack = ItemStack::new("minecraft:stick", 1);
        set(&mut stack, value);
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(named.cook(&context, "minecraft:smelting", &stack).is_none());
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert!(empty.cook(&context, "minecraft:smelting", &stack).is_none());
    }
    // Reload only rebinds tags: changed/removed definitions remain frozen and new
    // definitions do not appear until a fresh world load.
    let catalog = Catalog::load(root.path(), &overlay).unwrap();
    let reloaded_files = vec![
        (
            "damage_type/fixture:heat".into(),
            vec![br#"{"invalid":true}"#.to_vec()],
        ),
        (
            "damage_type/fixture:new".into(),
            vec![definition.to_string().into_bytes()],
        ),
        (
            "tags/fixture:damage_type/rebound".into(),
            vec![br#"{"values":["fixture:heat"]}"#.to_vec()],
        ),
    ];
    let reloaded_overlay = PackOverlay::new(&reloaded_files);
    let reloaded = catalog.reload_tags(root.path(), &reloaded_overlay).unwrap();
    items.bind_component_damage_catalog(&reloaded);
    let rebound = recipes(
        json!({"minecraft:damage_resistant":{"types":"#fixture:rebound"}}),
        false,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    set(&mut stack, tag(json!({"types":"#fixture:rebound"})));
    assert_eq!(
        rebound
            .cook(&context, "minecraft:smelting", &stack)
            .unwrap()
            .id,
        "fixture:resistant"
    );
    let frozen = recipes(
        json!({"minecraft:damage_resistant":{"types":"fixture:heat"}}),
        false,
        &items,
    );
    set(&mut stack, tag(json!({"types":["fixture:heat"]})));
    assert!(
        frozen
            .cook(&context, "minecraft:smelting", &stack)
            .is_some()
    );
    for types in ["#fixture:hot", "fixture:new"] {
        let refused = recipes(
            json!({"minecraft:damage_resistant":{"types":types}}),
            false,
            &items,
        );
        assert!(
            refused
                .cook(&context, "minecraft:smelting", &stack)
                .is_none()
        );
        assert_eq!(
            refused
                .cook(
                    &context,
                    "minecraft:smelting",
                    &ItemStack::new("minecraft:stone", 1)
                )
                .unwrap()
                .id,
            "fixture:sibling",
        );
    }
}
