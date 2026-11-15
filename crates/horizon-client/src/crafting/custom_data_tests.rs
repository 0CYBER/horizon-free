use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
use serde_json::{Value, json};

fn assert_custom_tags(data: &horizon_core::nbt::Compound) {
    use horizon_core::nbt::Tag;
    assert_eq!(data.get(&"integer".into()), Some(&Tag::Int(32768)));
    assert_eq!(data.get(&"long".into()), Some(&Tag::Long(2147483648)));
    assert_eq!(
        data.get(&"list".into()),
        Some(&Tag::List(vec![Tag::Byte(1), Tag::Short(128)]))
    );
    assert_eq!(
        data.get(&"nested".into()),
        Some(&Tag::compound([("enabled", Tag::Byte(1))]))
    );
}

struct TypedIngredient;
#[mod_api::__subscriptions]
impl mod_api::Hooks for TypedIngredient {
    fn ingredient(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::IngredientEvent<'_>,
    ) {
        assert_custom_tags(event.data);
        event.items.push("minecraft:emerald".into());
    }
}

#[test]
fn custom_ingredient_tags_reach_hook_and_matching_record() {
    let items = super::Items::for_tests(Default::default());
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("typed", Box::new(TypedIngredient))].into(),
        || {
            let context = LoadContext::new(
                DataLoadOrigin::ServerDataLoad,
                std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
                [],
            );
            let mut source = json!({"type": "typed:ingredient", "integer": 32768, "long": 2147483648i64,
            "list": [1, 128], "nested": {"enabled": true}});
            super::Ingredient::resolve(&mut source, &context);
            let parsed = super::Ingredient::parse(&source, &items).unwrap();
            let (_, data) = parsed.custom.unwrap();
            assert_custom_tags(&data);
            assert!(!data.contains_key(&"horizon:resolved_items".into()));

            // Refusal leaves the kind unanswered just as an unknown kind does;
            // no hook call should see the malformed metadata.
            source["nested"]["invalid"] = Value::Null;
            super::Ingredient::resolve(&mut source, &context);
            assert!(super::Ingredient::parse(&source, &items).is_none());
        },
    );
}

struct WidthIngredient;
#[mod_api::__subscriptions]
impl mod_api::Hooks for WidthIngredient {
    fn ingredient(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::IngredientEvent<'_>,
    ) {
        use horizon_core::nbt::Tag;
        let want = [
            ("flag", Tag::Byte(1)),
            ("one", Tag::Byte(1)),
            ("small", Tag::Short(300)),
            ("big", Tag::Int(100000)),
            ("half", Tag::Float(0.5f32.to_bits())),
            ("tenth", Tag::Double(0.1f64.to_bits())),
            ("whole", Tag::Byte(1)),
            ("exp", Tag::Short(1000)),
        ];
        for (key, tag) in want {
            assert_eq!(event.data.get(&key.into()), Some(&tag), "{key}");
        }
        event.items.push("minecraft:emerald".into());
    }
}

#[test]
fn custom_ingredient_scalars_reach_hook_with_narrowest_width() {
    let items = super::Items::for_tests(Default::default());
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("width", Box::new(WidthIngredient))].into(),
        || {
            let context = LoadContext::new(
                DataLoadOrigin::ServerDataLoad,
                std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
                [],
            );
            let mut source = json!({"type": "width:ingredient", "flag": true, "one": 1,
            "small": 300, "big": 100000, "half": 0.5, "tenth": 0.1, "whole": 1.0, "exp": 1e3});
            super::Ingredient::resolve(&mut source, &context);
            let parsed = super::Ingredient::parse(&source, &items).unwrap();
            assert_eq!(parsed.order, ["minecraft:emerald"]);
        },
    );
}

#[test]
fn ingredient_depth_128_matches_clones_and_drops_on_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let load = LoadContext::new(
                DataLoadOrigin::ServerDataLoad,
                std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
                [],
            );
            let context =
                load.loop_context(horizon_world::native_load_context::LoopSide::Server, []);
            let mut ingredient = super::Ingredient::of(vec!["sample:item".into()]);
            for _ in 1..128 {
                let mut parent = super::Ingredient::of(vec!["sample:item".into()]);
                parent.children = Some((super::Shape::Compound, vec![ingredient]));
                ingredient = parent;
            }
            let stack = horizon_items::items::ItemStack {
                item: "sample:item".into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            assert!(ingredient.test_in(&context, &stack));
            let cloned = ingredient.clone();
            assert!(cloned.test_in(&context, &stack));
            drop(cloned);
            drop(ingredient);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "requires prepared vanilla recipe corpus"]
fn vanilla_recipe_corpus_preserves_ingredient_membership_and_order() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client/26.3/data/minecraft/recipe");
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let json: Value = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        let mut sources = Vec::new();
        if let Some(value) = json.get("ingredient") {
            sources.push(value);
        }
        if let Some(values) = json.get("ingredients").and_then(Value::as_array) {
            sources.extend(values);
        }
        if let Some(values) = json.get("key").and_then(Value::as_object) {
            sources.extend(values.values());
        }
        for source in sources {
            let names: Vec<&str> = match source {
                Value::String(name) => vec![name],
                Value::Array(values) => values.iter().map(|v| v.as_str().unwrap()).collect(),
                _ => panic!("unexpected vanilla ingredient: {source}"),
            };
            // Independently expand tagged sources with sentinel members. This checks
            // the real corpus's source shapes, not the separate tag loader.
            let mut items = super::Items::for_tests(Default::default());
            let mut expected = Vec::new();
            for name in names {
                let id = if let Some(tag) = name.strip_prefix('#') {
                    items
                        .item_tag_order
                        .insert(tag.into(), vec!["fixture:tag_member".into()]);
                    "fixture:tag_member"
                } else {
                    name
                };
                if !expected.contains(&id.to_owned()) {
                    expected.push(id.to_owned());
                }
            }
            let parsed = super::Ingredient::parse(source, &items).unwrap();
            assert_eq!(parsed.order, expected, "{}", entry.path().display());
            assert_eq!(parsed.items, expected.into_iter().collect());
            assert!(parsed.children.is_none());
            assert!(parsed.custom.is_none());
            checked += 1;
        }
    }
    assert!(checked > 1000, "incomplete recipe corpus: {checked}");
}
