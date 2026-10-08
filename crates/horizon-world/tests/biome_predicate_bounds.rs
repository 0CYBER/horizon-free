use horizon_world::worldgen::{biome::BiomeId, biome_predicate::BiomePredicateBindings};
use std::collections::BTreeMap;
#[test]
fn biome_registry_identity_and_holder_set_bounds() {
    let names = BTreeMap::from([
        ("minecraft:plains".into(), BiomeId(0)),
        ("minecraft:desert".into(), BiomeId(1)),
    ]);
    let tags = BTreeMap::from([
        ("horizon:empty".into(), vec![]),
        (
            "horizon:both".into(),
            vec!["plains".into(), "desert".into(), "plains".into()],
        ),
    ]);
    let bindings = BiomePredicateBindings::new(&names, &tags).unwrap();
    assert!(
        bindings
            .resolve(r##""#horizon:empty""##)
            .unwrap()
            .is_empty()
    );
    assert_eq!(bindings.resolve(r##""#horizon:both""##).unwrap().len(), 2);
    assert!(bindings.resolve(r##"["#horizon:both"]"##).is_err());
    assert!(
        bindings
            .resolve(&serde_json::to_string(&vec!["plains"; 8193]).unwrap())
            .is_err()
    );
    let duplicate = BTreeMap::from([("plains".into(), BiomeId(0)), ("desert".into(), BiomeId(0))]);
    assert!(BiomePredicateBindings::new(&duplicate, &tags).is_err());
    let duplicate = BTreeMap::from([
        ("plains".into(), BiomeId(0)),
        ("minecraft:plains".into(), BiomeId(1)),
    ]);
    assert!(BiomePredicateBindings::new(&duplicate, &BTreeMap::new()).is_err());
    let unknown = BTreeMap::from([("tag".into(), vec!["missing".into()])]);
    assert!(BiomePredicateBindings::new(&names, &unknown).is_err());
}
