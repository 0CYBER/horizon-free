use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{block_feature::FeatureBlockBindings, rule_test::RuleTest},
};
use std::collections::{BTreeMap, BTreeSet};
fn registry() -> BlockRegistry {
    let names = [
        "minecraft:air",
        "minecraft:cave_air",
        "minecraft:void_air",
        "minecraft:stone",
    ];
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::Compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
                    .collect(),
            ),
        ),
    ]))
    .unwrap()
}
#[test]
fn rule_decoding_bounds_nested_and_branching_data() {
    let registry = registry();
    let bindings = FeatureBlockBindings::new(&registry, BTreeSet::new()).unwrap();
    let leaf = r#"{"predicate_type":"always_true"}"#;
    let mut nested = leaf.to_owned();
    for _ in 0..130 {
        nested = format!(r#"{{"predicate_type":"not","rule":{nested}}}"#);
    }
    assert!(RuleTest::from_json(&nested, &bindings).is_err());
    let branch = format!(
        r#"{{"predicate_type":"all_of","rules":[{}]}}"#,
        vec![leaf; 8192].join(",")
    );
    let tree = format!(
        r#"{{"predicate_type":"all_of","rules":[{}]}}"#,
        [branch.as_str(); 9].join(",")
    );
    assert!(RuleTest::from_json(&tree, &bindings).is_err());
}
#[test]
fn binding_rejects_unknown_tag_blocks_and_duplicate_canonical_tags() {
    let registry = registry();
    let tags = BTreeMap::from([("horizon:test".into(), vec!["missing".into()])]);
    assert!(FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags).is_err());
    let tags = BTreeMap::from([
        ("test".into(), vec!["stone".into()]),
        ("minecraft:test".into(), vec!["air".into()]),
    ]);
    assert!(FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags).is_err());
}
#[test]
fn matter_phase_matches_what_a_block_is_made_of() {
    // air, cave_air, void_air, stone, then water (level 0), rail, iron_chain.
    let mut states: Vec<Tag> = [
        "minecraft:air",
        "minecraft:cave_air",
        "minecraft:void_air",
        "minecraft:stone",
    ]
    .iter()
    .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
    .collect();
    states.push(Tag::compound([
        ("id", Tag::String("minecraft:water".into())),
        (
            "properties",
            Tag::compound([("level", Tag::String("0".into()))]),
        ),
    ]));
    for name in ["minecraft:rail", "minecraft:iron_chain"] {
        states.push(Tag::compound([("id", Tag::String(name.into()))]));
    }
    let names = [
        "minecraft:air",
        "minecraft:cave_air",
        "minecraft:void_air",
        "minecraft:stone",
        "minecraft:water",
        "minecraft:rail",
        "minecraft:iron_chain",
    ];
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states)),
        (
            "defaults",
            Tag::Compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
                    .collect(),
            ),
        ),
    ]))
    .unwrap();
    let bindings = FeatureBlockBindings::new(&registry, BTreeSet::new()).unwrap();
    let mut random = horizon_core::random::LegacyRandom::new(0);
    let matches = |phase: &str, invert: bool, random: &mut horizon_core::random::LegacyRandom| {
        let json = format!(
            r#"{{"predicate_type":"horizon:matter_phase","phase_to_test_for":"{phase}","invert_condition":{invert}}}"#
        );
        let test = RuleTest::from_json(&json, &bindings).unwrap();
        (0..names.len() as u32)
            .filter(|&id| test.test(horizon_world::registry::BlockStateId(id), [0; 3], random))
            .collect::<Vec<_>>()
    };
    assert_eq!(matches("AIR", false, &mut random), [0, 1, 2]);
    assert_eq!(matches("LIQUID", false, &mut random), [4]);
    assert_eq!(
        matches("AIR_RAIL_OR_CHAIN", false, &mut random),
        [0, 1, 2, 5, 6]
    );
    assert_eq!(
        matches("LIQUID_RAIL_OR_CHAIN", false, &mut random),
        [4, 5, 6]
    );
    assert_eq!(matches("AIR", true, &mut random), [3, 4, 5, 6]);
    // The phase is the enum's name as written; SOLID needs canOcclude.
    for json in [
        r#"{"predicate_type":"horizon:matter_phase","phase_to_test_for":"air"}"#,
        r#"{"predicate_type":"horizon:matter_phase","phase_to_test_for":"SOLID"}"#,
        r#"{"predicate_type":"horizon:matter_phase"}"#,
    ] {
        assert!(RuleTest::from_json(json, &bindings).is_err(), "{json}");
    }
}
