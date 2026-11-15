use super::{
    BlockStateDefinition, BlockStateFile, Variant, VariantSet,
    bind::{PropertySchema, PropertyValues, SelectedModels},
};
use std::collections::{BTreeMap, BTreeSet};

/// A file naming a definition type is kept whole for the type, with the type id read as an
/// identifier is; its `variants` are the type's to read, not vanilla's.
#[test]
fn a_definition_type_keeps_the_whole_file_for_its_type() {
    let text =
        r#"{"neoforge:definition_type":"lamps:by_power","on":"lamps:block/on","variants":7}"#;
    let BlockStateFile::Typed { kind, config } = BlockStateFile::from_json(text).unwrap() else {
        panic!("a typed file");
    };
    assert_eq!(kind, "lamps:by_power");
    assert_eq!(config["on"], "lamps:block/on");
    assert_eq!(config["variants"], 7);
}

/// The type id must be a string that reads as an identifier, or the file doesn't read.
#[test]
fn a_definition_type_that_isnt_an_identifier_is_refused() {
    for kind in ["3", r#""Lamps:By Power""#, "null", "{}"] {
        let text = format!(r#"{{"neoforge:definition_type":{kind}}}"#);
        assert!(BlockStateFile::from_json(&text).is_err(), "{kind}");
    }
}

/// Without the key, variants and multipart read exactly as vanilla's definitions do.
#[test]
fn a_file_without_a_type_reads_as_vanilla() {
    for text in [
        r#"{"variants":{"lit=true":{"model":"block/a","y":90},"lit=false":[{"model":"b"},{"model":"c","weight":3}]}}"#,
        r#"{"multipart":[{"apply":{"model":"block/post"}},{"when":{"OR":[{"north":"true"},{"east":"low|tall"}]},"apply":{"model":"block/side","uvlock":true}}]}"#,
    ] {
        assert_eq!(
            BlockStateFile::from_json(text).unwrap(),
            BlockStateFile::Vanilla(BlockStateDefinition::from_json(text).unwrap())
        );
    }
    let variants = r#"{"variants":{"":{"model":"block/a","x":270}}}"#;
    let BlockStateFile::Vanilla(definition) = BlockStateFile::from_json(variants).unwrap() else {
        panic!("vanilla");
    };
    assert_eq!(
        definition.variants.unwrap(),
        [(
            String::new(),
            VariantSet::Single(Variant {
                model: "minecraft:block/a".into(),
                rotations: [3, 0, 0],
                uv_lock: false,
            })
        )]
    );
    assert!(BlockStateFile::from_json(r#"{"variants":{}}"#).is_err());
    assert!(BlockStateFile::from_json(r#"{}"#).is_err());
}

/// Picks become one variant per distinct pick, bound by state; a state with no pick is unbound.
#[test]
fn picks_bind_each_state_to_its_variant_and_share_equal_ones() {
    let on = Variant::new("lamps:block/on", [0, 5, 0], false).unwrap();
    let off = Variant::new("block/off", [0; 3], true).unwrap();
    assert_eq!(on.rotations, [0, 1, 0], "five quarter turns are one");
    assert_eq!(off.model, "minecraft:block/off");
    let (definition, bound) = BlockStateDefinition::from_picks(vec![
        Some(off.clone()),
        None,
        Some(on.clone()),
        Some(off.clone()),
    ]);
    assert_eq!(
        bound,
        [
            Some(SelectedModels::Variant(0)),
            None,
            Some(SelectedModels::Variant(1)),
            Some(SelectedModels::Variant(0)),
        ]
    );
    let variants: Vec<_> = definition
        .variants
        .unwrap()
        .into_iter()
        .map(|(_, set)| set)
        .collect();
    assert_eq!(variants, [VariantSet::Single(off), VariantSet::Single(on)]);
    assert!(definition.multipart.is_none());
    assert!(Variant::new("Bad Model", [0; 3], false).is_err());
}

/// The vanilla binding is untouched: a variant key picks its states and a multipart its parts.
#[test]
fn vanilla_variants_and_multipart_bind_as_before() {
    let schema: PropertySchema = BTreeMap::from([(
        "lit".to_owned(),
        PropertyValues::Named(BTreeSet::from(["false".to_owned(), "true".to_owned()])),
    )]);
    let states: Vec<_> = ["false", "true"]
        .map(|lit| BTreeMap::from([("lit".to_owned(), lit.to_owned())]))
        .into();
    let variants = BlockStateDefinition::from_json(
        r#"{"variants":{"lit=true":{"model":"block/on"},"lit=false":{"model":"block/off"}}}"#,
    )
    .unwrap();
    assert_eq!(
        variants.bind(&schema, &states).unwrap(),
        [
            Some(SelectedModels::Variant(1)),
            Some(SelectedModels::Variant(0))
        ]
    );
    let multipart = BlockStateDefinition::from_json(
        r#"{"multipart":[{"apply":{"model":"block/base"}},{"when":{"lit":"true"},"apply":{"model":"block/flame"}}]}"#,
    )
    .unwrap();
    assert_eq!(
        multipart.bind(&schema, &states).unwrap(),
        [
            Some(SelectedModels::Multipart(vec![0])),
            Some(SelectedModels::Multipart(vec![0, 1]))
        ]
    );
}
