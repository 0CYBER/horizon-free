use super::*;
use crate::item::crafting::Shape;
use crate::world::snapshots::LootEntity;
use crate::{Ingredient, ItemPlace, RecipeSlotDisplay, Tag, Text};

fn round<T: Wire>(value: &T, initial: u16) {
    let mut out = Writer::new();
    out.put(value);
    assert_eq!(out.error(), None);
    let mut reader = Reader::new(out.bytes());
    reader.depth = initial;
    assert!(reader.take::<T>().is_ok());
    let mut reader = Reader::new(out.bytes());
    reader.depth = initial + 1;
    assert!(matches!(reader.take::<T>(), Err(WireError::Depth)));
}

#[test]
fn ingredient_at_limit_decodes_and_one_record_past_returns_wire_error() {
    let mut ingredient = Ingredient::of(vec!["minecraft:stone".into()]);
    for _ in 1..MAX_DEPTH {
        let mut parent = Ingredient::of(vec!["minecraft:stone".into()]);
        parent.children = Some((Shape::Compound, vec![ingredient]));
        ingredient = parent;
    }
    let mut at_limit = Writer::new();
    at_limit.put(&ingredient);
    assert_eq!(at_limit.error(), None);
    assert_eq!(
        Reader::decode::<Ingredient>(at_limit.bytes()).unwrap(),
        ingredient
    );
    // Write a complete extra record without charging its envelope to the
    // writer. This is hostile input, not a truncated failed encoder output.
    let parent = Ingredient::of(vec![]);
    let mut past_limit = Writer::new();
    past_limit.put(&parent.items);
    past_limit.put(&parent.order);
    past_limit.put(&parent.custom);
    past_limit.put(&Some((Shape::Compound, vec![ingredient])));
    past_limit.put(&parent.rule);
    past_limit.put(&parent.display);
    assert_eq!(past_limit.error(), None);
    assert!(matches!(
        Reader::decode::<Ingredient>(past_limit.bytes()),
        Err(WireError::Depth)
    ));
}

#[test]
fn structural_families_fit_the_guest_shadow_stack() {
    // The guest reserves 256 KiB; retain half for callers and transport plumbing.
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            // A raised limit is not safe merely because the depth counter still works.
            assert_eq!(MAX_DEPTH, 32);
            let mut ingredient = Ingredient::of(vec!["minecraft:stone".into()]);
            let mut display = RecipeSlotDisplay::Empty;
            for _ in 1..32 {
                let mut parent = Ingredient::of(vec!["minecraft:stone".into()]);
                parent.children = Some((Shape::Compound, vec![ingredient]));
                ingredient = parent;
                display = RecipeSlotDisplay::Composite(vec![display]);
            }
            round(&ingredient, 0);
            round(&display, 0);
            let mut place = ItemPlace::ItemEntity { id: 1 };
            let mut entity = LootEntity::default();
            for _ in 0..15 {
                place = ItemPlace::Nested {
                    parent: Box::new(place),
                    slot: 0,
                };
                entity = LootEntity {
                    vehicle: Some(Box::new(entity)),
                    ..Default::default()
                };
            }
            round(&Box::new(place), 0);
            round(&Box::new(entity), 0);
            // These public SCC members no longer recurse structurally: every Text
            // edge crosses through a flat table. Their own envelope still guards depth.
            round(&crate::text::TextContent::Literal(String::new()), 31);
            round(&crate::text::TextStyle::default(), 31);
            round(
                &crate::text::TextHover::Item {
                    id: String::new(),
                    count: 1,
                    components: None,
                },
                31,
            );
            round(
                &crate::text::TextObject::Atlas {
                    atlas: String::new(),
                    sprite: String::new(),
                    fallback: None,
                },
                31,
            );
            let mut parent = Ingredient::of(vec![]);
            parent.children = Some((Shape::Compound, vec![ingredient]));
            let mut out = Writer::new();
            out.put(&parent);
            assert_eq!(out.error(), Some(WireError::Depth));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn vanilla_trees_keep_their_depth_on_the_guest_shadow_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for compound in [false, true] {
                let mut tag = Tag::Int(7);
                for _ in 0..512 {
                    tag = if compound {
                        Tag::compound([("k", tag)])
                    } else {
                        Tag::List(vec![tag])
                    };
                }
                let mut out = Writer::new();
                out.put(&tag);
                assert_eq!(out.error(), None);
                let read = Reader::decode::<Tag>(out.bytes()).unwrap();
                horizon_core::nbt::discard_tag(read);
                let mut bytes = if compound {
                    vec![10, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 107, 0]
                } else {
                    vec![9, 0, 0, 0, 1, 0, 0, 0]
                };
                bytes.extend_from_slice(out.bytes());
                assert!(matches!(
                    Reader::decode::<Tag>(&bytes),
                    Err(WireError::Depth)
                ));
                horizon_core::nbt::discard_tag(tag);
            }
            let mut text = Text::literal("leaf");
            for _ in 1..512 {
                text = Text::literal("").append(text);
            }
            let mut out = Writer::new();
            out.put(&text);
            assert_eq!(out.error(), None);
            let mut form = crate::text::TextForm::try_from(&text).unwrap();
            let mut extra =
                crate::text::TextNode::new(crate::text::TextNodeContent::Literal(String::new()));
            extra.parent = Some(511);
            form.nodes.push(extra);
            let mut invalid = Writer::new();
            invalid.put(&form);
            assert!(matches!(
                Reader::decode::<Text>(invalid.bytes()),
                Err(WireError::Invalid(_))
            ));
            // Drop is outside Wire's ownership; intentionally discard a deep tree iteratively.
            let decoded = Reader::decode::<Text>(out.bytes()).unwrap();
            let text = Text::literal("").append(text);
            let mut too_deep = Writer::new();
            too_deep.put(&text);
            assert_eq!(too_deep.error(), Some(WireError::Depth));
            let mut pending = vec![text, decoded];
            while let Some(mut text) = pending.pop() {
                pending.append(&mut text.children);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn json_backed_codecs_preflight_before_serde_recursion() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let quoted = serde_json::Value::String("[\\\"{".repeat(1024));
            let mut quoted_bytes = Writer::new();
            quoted_bytes.put(&quoted);
            assert_eq!(
                Reader::decode::<serde_json::Value>(quoted_bytes.bytes()).unwrap(),
                quoted
            );
            let mut value = serde_json::Value::Null;
            for _ in 0..32 {
                value = serde_json::Value::Array(vec![value]);
            }
            let mut out = Writer::new();
            out.put(&value);
            assert_eq!(out.error(), None);
            let mut map_at_limit = serde_json::Map::new();
            let mut child = serde_json::Value::Null;
            for _ in 0..31 {
                child = serde_json::Value::Array(vec![child]);
            }
            map_at_limit.insert("leaf".into(), child);
            let mut map_bytes = Writer::new();
            map_bytes.put(&map_at_limit);
            assert_eq!(map_bytes.error(), None);
            assert!(
                Reader::decode::<serde_json::Map<String, serde_json::Value>>(map_bytes.bytes())
                    .is_ok()
            );
            let decoded = Reader::decode::<serde_json::Value>(out.bytes()).unwrap();
            drop(decoded);
            value = serde_json::Value::Array(vec![value]);
            let mut out = Writer::new();
            out.put(&value);
            assert_eq!(out.error(), Some(WireError::Depth));
            let text = format!("{}null{}", "[".repeat(33), "]".repeat(33));
            let mut raw = Writer::new();
            raw.put(&text);
            assert!(matches!(
                Reader::decode::<serde_json::Value>(raw.bytes()),
                Err(WireError::Depth)
            ));
            for _ in 33..1024 {
                value = serde_json::Value::Array(vec![value]);
            }
            let mut map = serde_json::Map::new();
            map.insert("deep".into(), value);
            let mut out = Writer::new();
            out.put(&map);
            assert_eq!(out.error(), Some(WireError::Depth));
            let mut pending = vec![serde_json::Value::Object(map)];
            while let Some(value) = pending.pop() {
                match value {
                    serde_json::Value::Array(values) => pending.extend(values),
                    serde_json::Value::Object(values) => pending.extend(values.into_values()),
                    _ => {}
                }
            }
            let mut model = crate::ModelPart::new(vec![], vec![], Default::default());
            for _ in 0..10 {
                model = crate::ModelPart::new(
                    vec![],
                    vec![("child".into(), model)],
                    Default::default(),
                );
            }
            let mut valid_model = Writer::new();
            valid_model.put(&model);
            assert_eq!(valid_model.error(), None);
            let _ = Reader::decode::<crate::ModelPart>(valid_model.bytes()).unwrap();
            for depth in 10..1024 {
                if depth == 11 {
                    let mut refused = Writer::new();
                    refused.put(&model);
                    assert_eq!(refused.error(), Some(WireError::Depth));
                }
                model = crate::ModelPart::new(
                    vec![],
                    vec![("child".into(), model)],
                    Default::default(),
                );
            }
            let mut out = Writer::new();
            out.put(&model);
            assert_eq!(out.error(), Some(WireError::Depth));
            let mut parts = vec![model];
            while let Some(mut part) = parts.pop() {
                for (_, child) in part.children_mut() {
                    parts.push(std::mem::replace(
                        child,
                        crate::ModelPart::new(vec![], vec![], Default::default()),
                    ));
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn ingredient_shape_contract_keeps_existing_tags_and_adds_custom_display_at_three() {
    for (shape, bytes) in [
        (Shape::Compound, [0, 0, 0, 0]),
        (Shape::Intersection, [1, 0, 0, 0]),
        (Shape::Difference, [2, 0, 0, 0]),
        (Shape::CustomDisplay, [3, 0, 0, 0]),
    ] {
        let mut out = Writer::new();
        out.put(&shape);
        assert_eq!(out.bytes(), bytes);
        assert_eq!(Reader::decode::<Shape>(&bytes).unwrap(), shape);
    }
}

#[test]
fn explosion_interactions_cross_in_vanillas_order() {
    use crate::world::types::ExplosionInteraction;
    for (interaction, bytes) in [
        (ExplosionInteraction::None, [0, 0, 0, 0]),
        (ExplosionInteraction::Block, [1, 0, 0, 0]),
        (ExplosionInteraction::Mob, [2, 0, 0, 0]),
        (ExplosionInteraction::Tnt, [3, 0, 0, 0]),
        (ExplosionInteraction::Trigger, [4, 0, 0, 0]),
    ] {
        let mut out = Writer::new();
        out.put(&interaction);
        assert_eq!(out.bytes(), bytes);
        assert_eq!(
            Reader::decode::<ExplosionInteraction>(&bytes).unwrap(),
            interaction
        );
    }
}

#[test]
fn boxed_ingredient_metadata_preserves_its_value_encoding() {
    let rule = Some(crate::item::crafting::IngredientRule::Components {
        patch: [("minecraft:damage".into(), Tag::Int(5))].into(),
        removed: vec!["minecraft:enchantments".into()],
        strict: true,
    });
    let mut inline = Writer::new();
    inline.put(&rule);
    let mut boxed = Writer::new();
    boxed.put(&rule.map(Box::new));
    assert_eq!(inline.bytes(), boxed.bytes());
    let display = Some(horizon_core::nbt::Compound::from([
        ("type".into(), Tag::String("minecraft:item".into())),
        ("item".into(), Tag::String("minecraft:diamond".into())),
    ]));
    let mut inline = Writer::new();
    inline.put(&display);
    let mut boxed = Writer::new();
    boxed.put(&display.map(Box::new));
    assert_eq!(inline.bytes(), boxed.bytes());
}
