use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::state_provider::{BlockStateProvider, StateProviderBindings, StateProviderCatalog},
};
use std::collections::{BTreeMap, BTreeSet};
#[test]
fn bound_recursive_providers_and_registry_lists_before_sampling() {
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![Tag::compound([(
                "id",
                Tag::String("minecraft:stone".into()),
            )])]),
        ),
        (
            "defaults",
            Tag::compound([("minecraft:stone", Tag::Int(0))]),
        ),
    ]))
    .unwrap();
    let bindings =
        StateProviderBindings::new(&registry, BTreeSet::new(), &BTreeMap::new()).unwrap();
    let mut source = r#"{"id":"stone"}"#.to_owned();
    for _ in 0..140 {
        source = format!(r#"{{"type":"rotated","state":{source}}}"#);
    }
    assert!(BlockStateProvider::from_json(&source, &bindings).is_err());
    let source = serde_json::json!({"type":"random_block","blocks":vec!["stone";8193]}).to_string();
    assert!(BlockStateProvider::from_json(&source, &bindings).is_err());
    let rule = serde_json::json!({"if_true":{"type":"true"},"then":{"id":"stone"}});
    let valid = serde_json::json!({"type":"rule_based","rules":vec![rule.clone();8192]});
    assert!(BlockStateProvider::from_json(&valid.to_string(), &bindings).is_ok());
    let oversized = serde_json::json!({"type":"rule_based","rules":vec![rule;8193]});
    assert!(BlockStateProvider::from_json(&oversized.to_string(), &bindings).is_err());
    // Every individual list fits, but the aggregate predicate/provider tree
    // must share a work budget rather than resetting it for each rule.
    let branching = serde_json::json!({"if_true":{"type":"all_of","predicates":vec![serde_json::json!({"type":"true"});7]},"then":{"id":"stone"}});
    let oversized = serde_json::json!({"type":"rule_based","rules":vec![branching;8192]});
    assert!(BlockStateProvider::from_json(&oversized.to_string(), &bindings).is_err());
    // Content mods' blocks join shared tags: the cap is 2^20 blocks.
    let tags = BTreeMap::from([("horizon:large".into(), vec!["stone".into(); 8193])]);
    assert!(StateProviderBindings::new(&registry, BTreeSet::new(), &tags).is_ok());
    let tags = BTreeMap::from([(
        "horizon:oversized".into(),
        vec!["stone".into(); (1 << 20) + 1],
    )]);
    assert!(StateProviderBindings::new(&registry, BTreeSet::new(), &tags).is_err());
    let tags = BTreeMap::from([("tag".into(), vec![]), ("minecraft:tag".into(), vec![])]);
    assert!(StateProviderBindings::new(&registry, BTreeSet::new(), &tags).is_err());
    let definitions = BTreeMap::from([
        (
            "horizon:cycle".into(),
            r#"{"type":"copy_properties","source":"horizon:cycle"}"#.into(),
        ),
        ("horizon:valid".into(), r#"{"id":"stone"}"#.into()),
        ("horizon:invalid".into(), r#"{"type":"missing"}"#.into()),
        ("horizon:alias".into(), r#""horizon:valid""#.into()),
    ]);
    let catalog = StateProviderCatalog::from_resources(&definitions).unwrap();
    assert!(catalog.provider(r#""horizon:valid""#, &bindings).is_ok());
    for name in [
        "horizon:cycle",
        "horizon:invalid",
        "horizon:alias",
        "horizon:missing",
    ] {
        assert!(
            catalog
                .provider(&serde_json::to_string(name).unwrap(), &bindings)
                .is_err()
        );
    }
    assert!(
        catalog
            .direct_provider(r#""horizon:valid""#, &bindings)
            .is_err()
    );
    let duplicate = BTreeMap::from([
        ("stone".into(), "{}".into()),
        ("minecraft:stone".into(), "{}".into()),
    ]);
    assert!(StateProviderCatalog::from_resources(&duplicate).is_err());
}
