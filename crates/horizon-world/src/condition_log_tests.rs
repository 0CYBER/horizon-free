//! Thread-local capture follows the repository's residency diagnostic tests.
use super::*;
use serde_json::json;
use std::cell::RefCell;
thread_local! {
    static RECORDS: RefCell<Vec<(log::Level, String)>> = const { RefCell::new(Vec::new()) };
}
struct Capture;
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        RECORDS
            .with_borrow_mut(|records| records.push((record.level(), record.args().to_string())));
    }
    fn flush(&self) {}
}
static CAPTURE: Capture = Capture;
static INSTALL: std::sync::Once = std::sync::Once::new();
#[test]
fn foreign_unknown_after_true_is_absent_and_logged_once_at_error_level() {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Error);
    });
    for registered in [false, true] {
        let conditions = if registered {
            LoadConditions::new(["othermod"]).with_lookup(
                BTreeMap::from([("testmod".into(), "testmod".into())]),
                |name, _| (name == "testmod:flag").then_some(Ok(true)),
            )
        } else {
            LoadConditions::new([])
        };
        RECORDS.with_borrow_mut(Vec::clear);
        let file = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
            {"type":"neoforge:true"},{"type":"othermod:unknown"}
        ]}]});
        for _ in 0..2 {
            assert!(!conditions.allows_file(&file, "data/testmod/recipe/foreign_or_true.json"));
        }
        assert_eq!(conditions.check_errors(), Ok(()));
        RECORDS.with_borrow(|records| assert_eq!(records.as_slice(), &[(log::Level::Error,
            "data/testmod/recipe/foreign_or_true.json: unknown load conditions [othermod:unknown]; file left out".into())]));
        RECORDS.with_borrow_mut(Vec::clear);
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
        RECORDS.with_borrow(|records| {
            assert_eq!(records.as_slice(), &[(log::Level::Error,
                "data/testmod/recipe/foreign_nested.json: unknown load conditions [othermod:second, othermod:unknown]; file left out".into())]);
            assert_eq!(records[0].1.matches("data/testmod/recipe/foreign_nested.json").count(), 1);
        });
    }
    // The const vanilla view has no candidate diagnostic store: cover the
    // fallback emission branch as well as the stored-candidate branch above.
    RECORDS.with_borrow_mut(Vec::clear);
    let file = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
        {"type":"neoforge:true"},{"type":"othermod:unknown"},{"type":"othermod:unknown"}
    ]}]});
    assert!(
        !LoadConditions::vanilla().allows_file(&file, "data/testmod/recipe/foreign_static.json")
    );
    RECORDS.with_borrow(|records| assert_eq!(records.as_slice(), &[(log::Level::Error,
        "data/testmod/recipe/foreign_static.json: unknown load conditions [othermod:unknown]; file left out".into())]));
}
