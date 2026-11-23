//! Lines read from the crate's one test logger (`crate::test_log`), this thread's only.
use super::*;
use crate::test_log;
use serde_json::json;
#[test]
fn foreign_unknown_after_true_is_absent_and_logged_once_at_error_level() {
    test_log::capture();
    for registered in [false, true] {
        let conditions = if registered {
            LoadConditions::new(["othermod"]).with_lookup(
                BTreeMap::from([("testmod".into(), "testmod".into())]),
                |name, _| (name == "testmod:flag").then_some(Ok(true)),
            )
        } else {
            LoadConditions::new([])
        };
        test_log::clear();
        let file = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
            {"type":"neoforge:true"},{"type":"othermod:unknown"}
        ]}]});
        for _ in 0..2 {
            assert!(!conditions.allows_file(&file, "data/testmod/recipe/foreign_or_true.json"));
        }
        assert_eq!(conditions.check_errors(), Ok(()));
        test_log::with_records(|records| {
            assert_eq!(records.as_slice(), &[(log::Level::Error,
            "data/testmod/recipe/foreign_or_true.json: unknown load conditions [othermod:unknown]; file left out".into())])
        });
        test_log::clear();
        let nested = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
            {"type":"neoforge:true"},
            {"type":"neoforge:and","values":[
                {"type":"neoforge:mod_loaded","modid":"absent"},
                {"type":"neoforge:not","value":{"type":"neoforge:not","value":{"type":"othermod:unknown"}}},
                {"type":"othermod:unknown"},{"type":"othermod:second"}
            ]}
        ]}]});
        for _ in 0..2 {
            assert!(!conditions.allows_file(&nested, "data/testmod/recipe/foreign_nested.json"));
        }
        test_log::with_records(|records| {
            assert_eq!(records.as_slice(), &[(log::Level::Error,
                "data/testmod/recipe/foreign_nested.json: unknown load conditions [othermod:second, othermod:unknown]; file left out".into())]);
            assert_eq!(
                records[0]
                    .1
                    .matches("data/testmod/recipe/foreign_nested.json")
                    .count(),
                1
            );
        });
    }
    // The const vanilla view has no candidate diagnostic store: cover the
    // fallback emission branch as well as the stored-candidate branch above.
    test_log::clear();
    let file = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
        {"type":"neoforge:true"},{"type":"othermod:unknown"},{"type":"othermod:unknown"}
    ]}]});
    assert!(
        !LoadConditions::vanilla().allows_file(&file, "data/testmod/recipe/foreign_static.json")
    );
    test_log::with_records(|records| {
        assert_eq!(records.as_slice(), &[(log::Level::Error,
        "data/testmod/recipe/foreign_static.json: unknown load conditions [othermod:unknown]; file left out".into())])
    });
}
