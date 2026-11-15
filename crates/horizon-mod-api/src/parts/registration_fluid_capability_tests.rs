#[cfg(test)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestReader;
    use crate::{DataLoadOrigin, OwningLoad};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// The load the game's recipe pass builds: API reader fixture, with the
    /// tags the game ships in `worldgen/fluid_tags.json` — water holds the
    /// flowing fluid too.
    fn load() -> TestReader {
        crate::__private::install(Default::default());
        crate::__private::install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| {
            false
        })
        .unwrap();
        mod_vocab::__private::resolve_fluids(
            &[
                "minecraft:empty",
                "minecraft:water",
                "minecraft:flowing_water",
                "minecraft:lava",
                "minecraft:flowing_lava",
                "minecraft:milk",
            ],
            &[],
        )
        .unwrap();
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:water".to_owned(),
            vec![
                "minecraft:water".to_owned(),
                "minecraft:flowing_water".to_owned(),
            ],
        );
        tags.insert("c:lava".to_owned(), vec!["minecraft:lava".to_owned()]);
        tags.insert("fixture:empty".to_owned(), vec![]);
        TestReader {
            registries: Default::default(),
            tags: tags
                .into_iter()
                .map(|(k, v)| (k, Arc::from(v)))
                .collect::<Vec<_>>()
                .into(),
        }
    }
    fn parse(s: &str) -> Option<FluidIngredient> {
        let load = load();
        FluidIngredient::from_json(
            &serde_json::from_str(s).unwrap(),
            &OwningLoad::new(DataLoadOrigin::ServerDataLoad, &load),
        )
        .ok()
    }
    fn stack(fluid: &str, amount: u32) -> FluidStack {
        FluidStack::new(mod_vocab::__private::fluid_key(fluid).unwrap(), amount)
    }

    #[test]
    fn every_expression_leaf_is_validated_before_set_operations() {
        let load = load();
        let context = OwningLoad::new(DataLoadOrigin::ServerDataLoad, &load);
        for json in [
            r#"{"type":"neoforge:difference","base":"minecraft:water","subtracted":"absent:fluid"}"#,
            r#"{"type":"neoforge:intersection","children":[["minecraft:water","absent:fluid"],"minecraft:water"]}"#,
        ] {
            assert!(
                matches!(FluidIngredient::from_json(&serde_json::from_str(json).unwrap(), &context), Err(FluidIngredientError::UnknownFluid(name)) if name == "absent:fluid")
            );
        }
        let water = FluidIngredient::from_json(&serde_json::json!("water"), &context).unwrap();
        assert!(water.test(&stack("minecraft:water", 1)));
    }

    #[test]
    fn empty_fluid_descendants_remain_invalid() {
        let reader = load();
        let context = OwningLoad::new(DataLoadOrigin::ServerDataLoad, &reader);
        for json in [
            serde_json::json!("#fixture:empty"),
            serde_json::json!({"type":"neoforge:compound","children":["#fixture:empty"]}),
            serde_json::json!({"type":"neoforge:difference","base":"minecraft:water",
                "subtracted":"#fixture:empty"}),
            serde_json::json!({"type":"neoforge:compound","children":[
                "minecraft:water", {"type":"neoforge:compound","children":["#fixture:empty"]}]}),
        ] {
            assert!(matches!(
                FluidIngredient::from_json(&json, &context),
                Err(FluidIngredientError::Invalid)
            ));
        }
    }

    /// A single fluid matches itself and nothing else.
    #[test]
    fn a_single_fluid_matches_itself() {
        let single = parse(r#""minecraft:water""#).unwrap();
        assert!(single.test(&stack("minecraft:water", 100)));
        assert!(!single.test(&stack("minecraft:lava", 100)));
        assert_eq!(
            single.fluids(),
            [stack("minecraft:water", 1).key().unwrap()]
        );
    }

    /// A `#tag` resolves through the fluid tags, keeping their order; a
    /// fluid outside it and a tag that isn't loaded answer nothing.
    #[test]
    fn a_tag_resolves_through_the_fluid_tags() {
        let tag = parse(r##""#minecraft:water""##).unwrap();
        assert_eq!(
            tag.fluids(),
            [
                stack("minecraft:water", 1).key().unwrap(),
                stack("minecraft:flowing_water", 1).key().unwrap()
            ]
        );
        assert!(tag.test(&stack("minecraft:flowing_water", 1)));
        assert!(!tag.test(&stack("minecraft:lava", 1)));
        assert!(parse(r##""#minecraft:unloaded""##).is_none());
    }

    /// A list keeps first-seen order and drops the rest.
    #[test]
    fn a_list_dedups_keeping_order() {
        let list = parse(r##"["minecraft:water","#minecraft:water","minecraft:lava"]"##).unwrap();
        assert_eq!(
            list.fluids(),
            [
                stack("minecraft:water", 1).key().unwrap(),
                stack("minecraft:flowing_water", 1).key().unwrap(),
                stack("minecraft:lava", 1).key().unwrap()
            ]
        );
        assert!(parse(r#"[]"#).is_none());
    }

    /// The shapes nest: compound answers any child, difference the base
    /// without the subtracted — and a kind outside them fails the parse,
    /// as fluids have no custom ingredient hook.
    #[test]
    fn the_shapes_nest_and_a_custom_kind_fails() {
        let compound = parse(
            r##"{"neoforge:ingredient_type":"neoforge:compound","children":["minecraft:lava","#minecraft:water"]}"##,
        )
        .unwrap();
        assert!(compound.test(&stack("minecraft:water", 1)));
        assert!(compound.test(&stack("minecraft:lava", 1)));
        assert!(!compound.test(&stack("minecraft:milk", 1)));
        let intersection = parse(
            r##"{"type":"neoforge:intersection","children":["#minecraft:water",["minecraft:water","minecraft:lava"]]}"##,
        )
        .unwrap();
        assert!(intersection.test(&stack("minecraft:water", 1)));
        assert!(!intersection.test(&stack("minecraft:lava", 1)));
        let difference = parse(
            r##"{"type":"neoforge:difference","base":"#minecraft:water","subtracted":"minecraft:flowing_water"}"##,
        )
        .unwrap();
        assert_eq!(
            difference.fluids(),
            [stack("minecraft:water", 1).key().unwrap()]
        );
        assert!(!difference.test(&stack("minecraft:flowing_water", 1)));
        // A mod's own kind, an unknown minecraft kind and a kindless
        // object all fail the file.
        assert!(parse(r#"{"type":"mymod:hot","heat":9}"#).is_none());
        assert!(parse(r#"{"type":"minecraft:future_shape"}"#).is_none());
        assert!(parse(r#"{"fluid":"minecraft:water"}"#).is_none());
    }

    /// The sized ingredient asks an amount: the stack must match and carry
    /// at least that much; `amount` defaults to a bucket and must be
    /// positive.
    #[test]
    fn the_sized_ingredient_checks_the_amount() {
        let load = load();
        let sized = |s: &str| {
            SizedFluidIngredient::from_json(
                &serde_json::from_str(s).unwrap(),
                &OwningLoad::new(DataLoadOrigin::ServerDataLoad, &load),
            )
        };
        let half = sized(r#"{"ingredient":"minecraft:water","amount":500}"#).unwrap();
        assert!(!half.test(&stack("minecraft:water", 499)));
        assert!(half.test(&stack("minecraft:water", 500)));
        assert!(half.test(&stack("minecraft:water", 1000)));
        assert!(!half.test(&stack("minecraft:lava", 1000)));
        // A bucket by default (`FluidType.BUCKET_VOLUME`).
        let bucket = sized(r#"{"ingredient":"minecraft:water"}"#).unwrap();
        assert_eq!(bucket.amount, 1000);
        assert!(!bucket.test(&stack("minecraft:water", 999)));
        assert!(bucket.test(&stack("minecraft:water", 1000)));
        // A positive amount, as `ExtraCodecs.POSITIVE_INT` asks.
        assert!(sized(r#"{"ingredient":"minecraft:water","amount":0}"#).is_none());
        assert!(sized(r#"{"ingredient":"minecraft:water","amount":-1}"#).is_none());
        assert!(sized(r#"{"ingredient":"minecraft:water","amount":1.5}"#).is_none());
        assert!(sized(r##"{"ingredient":"#minecraft:unloaded"}"##).is_none());
        assert!(sized(r#"{"ingredient":[]}"#).is_none());
    }

    /// The same walk parses item ingredients unchanged: the fluid kinds
    /// share its code without touching its answers.
    #[test]
    fn item_ingredients_still_parse_the_same() {
        let items = ();
        // Supply the resolved ids fixture, not a second JSON/NBT converter.
        let parse = |v: &serde_json::Value, _items: &()| {
            crate::__private::ingredient_parser::WalkedIngredient::walk(
                v,
                &HashMap::new(),
                &|_| None,
                &crate::__private::ingredient_parser::resolved_items,
                &|data| {
                    let ids = data.get("horizon:resolved_items")?.as_array()?;
                    let ids = ids
                        .iter()
                        .map(|v| horizon_core::nbt::Tag::String(v.as_str().unwrap().into()))
                        .collect();
                    Some(horizon_core::nbt::Compound::from([(
                        "horizon:resolved_items".into(),
                        horizon_core::nbt::Tag::List(ids),
                    )]))
                },
            )
        };
        let json = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let stone = parse(&json(r#""minecraft:stone""#), &items).unwrap();
        assert_eq!(stone.order, ["minecraft:stone"]);
        let union = parse(
            &json(r#"{"type":"neoforge:compound","children":["minecraft:oak_planks","minecraft:stone"]}"#),
            &items,
        )
        .unwrap();
        assert_eq!(union.order, ["minecraft:oak_planks", "minecraft:stone"]);
        assert!(parse(&json(r#"{"type":"mymod:aspect"}"#), &items).is_none());
        // A mod's kind nested in a shape brings the list its hook left at
        // load into the shape's ids, as on main before the shared walk.
        let aspect = r#"{"type":"mymod:aspect","horizon:resolved_items":["minecraft:oak_planks","minecraft:stone"]}"#;
        let union = parse(
            &json(&format!(
                r#"{{"type":"neoforge:compound","children":[{aspect},"minecraft:dirt"]}}"#
            )),
            &items,
        )
        .unwrap();
        assert_eq!(
            union.order,
            ["minecraft:oak_planks", "minecraft:stone", "minecraft:dirt"]
        );
        let both = parse(
            &json(&format!(
                r#"{{"type":"neoforge:intersection","children":[{aspect},["minecraft:stone","minecraft:dirt"]]}}"#
            )),
            &items,
        )
        .unwrap();
        assert_eq!(both.order, ["minecraft:stone"]);
        assert!(parse(&json(r#"{"type":"minecraft:future_shape"}"#), &items).is_none());
    }
}
