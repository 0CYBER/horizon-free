use super::*;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn registered(count: Arc<AtomicUsize>) -> LoadConditions {
    LoadConditions::new(["testmod", "othermod"]).with_lookup(
        BTreeMap::from([("testmod".into(), "testmod".into())]),
        move |name, args| {
            (name == "testmod:flag").then(|| {
                count.fetch_add(1, Ordering::SeqCst);
                args["value"]
                    .as_bool()
                    .ok_or_else(|| "value must be a boolean".into())
            })
        },
    )
}
fn variants(name: &str) -> Vec<Value> {
    let leaf = json!({"type":name});
    vec![
        leaf.clone(),
        json!({"type":"neoforge:not", "value":{"type":"neoforge:not", "value":leaf}}),
        json!({"type":"neoforge:and", "values":[{"type":"neoforge:mod_loaded","modid":"absent"},leaf]}),
        json!({"type":"neoforge:or", "values":[{"type":"neoforge:true"},leaf]}),
    ]
}
#[test]
fn owned_unknown_conditions_refuse_at_every_depth_and_foreign_names_report_once() {
    for (i, node) in variants("testmod:nope").into_iter().enumerate() {
        let conditions = registered(Arc::default());
        let path = format!("data/testmod/recipe/owned_{i}.json");
        assert!(!conditions.allows_file(&json!({"neoforge:conditions":[node]}), &path));
        let error = conditions.check_errors().unwrap_err();
        assert!(error.contains(&path), "{error}");
        assert!(error.contains("testmod:nope"), "{error}");
        assert!(error.contains("mod testmod"), "{error}");
    }
    for node in variants("othermod:thing") {
        let conditions = registered(Arc::default());
        let file = json!({"neoforge:conditions":[node.clone(), node, {"type":"othermod:second"}]});
        for _ in 0..2 {
            assert!(!conditions.allows_file(&file, "data/testmod/recipe/foreign.json"));
        }
        assert_eq!(conditions.check_errors(), Ok(()));
        assert_eq!(
            conditions.foreign_errors(),
            [
                "data/testmod/recipe/foreign.json: unknown load conditions [othermod:second, othermod:thing]; file left out"
            ]
        );
        assert_eq!(
            conditions.foreign_errors()[0]
                .matches("data/testmod/recipe/foreign.json")
                .count(),
            1
        );
    }
}
#[test]
fn registered_conditions_cache_each_entry_and_new_reload_evaluates_again() {
    let count = Arc::new(AtomicUsize::new(0));
    let yes = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
    let no = json!({"neoforge:conditions":[{"type":"testmod:flag","value":false}]});
    for expected in [2, 4] {
        let conditions = registered(count.clone());
        for _ in 0..3 {
            assert!(conditions.allows_id(&yes, "recipe", "testmod:enabled"));
            assert!(!conditions.allows_id(&no, "recipe", "testmod:disabled"));
        }
        assert_eq!(count.load(Ordering::SeqCst), expected);
    }
    let conditions = registered(count);
    assert!(conditions.allows_file(&json!({"neoforge:conditions":[{"type":"neoforge:and","values":[{"type":"testmod:flag","value":true},{"type":"neoforge:or","values":[{"type":"testmod:flag","value":false},{"type":"neoforge:true"}]}]}]}), "nested.json"));
    assert!(!conditions.allows_file(
        &json!({"neoforge:conditions":[{"type":"testmod:flag","value":"bad"}]}),
        "bad.json"
    ));
    assert_eq!(
        conditions.check_errors().unwrap_err(),
        "bad.json: value must be a boolean"
    );
}
#[test]
fn loaded_mod_ids_alone_do_not_own_condition_namespaces() {
    let conditions = LoadConditions::new(["testmod"]);
    for name in ["testmod:nope", "othermod:thing"] {
        for node in variants(name) {
            let file = json!({"neoforge:conditions":[node]});
            assert!(!conditions.allows_file(&file, "foreign.json"));
            assert!(!conditions_hold(&file, &|_| false));
            assert_eq!(conditions.check_errors(), Ok(()));
        }
    }
    assert_eq!(conditions.foreign_errors().len(), 1);
}

#[test]
fn no_mod_conditions_do_not_construct_file_context_for_known_nodes() {
    let conditions = LoadConditions::new([]);
    for (node, expected) in [
        (json!({"type":"neoforge:true"}), true),
        (
            json!({"type":"neoforge:mod_loaded","modid":"absent"}),
            false,
        ),
        (
            json!({"type":"neoforge:not","value":{"type":"neoforge:mod_loaded","modid":"absent"}}),
            true,
        ),
    ] {
        assert_eq!(
            conditions.allows_context(&json!({"neoforge:conditions":[node]}), || panic!(
                "fast path formatted a file context"
            )),
            expected
        );
    }
}
#[test]
fn concurrent_registered_readers_evaluate_each_entry_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let conditions = registered(count.clone());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                assert!(conditions.allows_file(
                    &json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]}),
                    "concurrent.json"
                ))
            });
        }
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn inventory_changes_invalidate_condition_answers_and_errors_without_mutating_old_view() {
    let old = registered(Arc::default());
    let file = json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"create"}]});
    assert!(!old.allows_file(&file, "inventory.json"));
    let new = old.with_mod_inventory(["create"]);
    assert!(new.allows_file(&file, "inventory.json"));
    assert!(!old.allows_file(&file, "inventory.json"));
    assert!(!old.allows_file(
        &json!({"neoforge:conditions":[{"type":"testmod:nope"}]}),
        "bad.json"
    ));
    let changed = old.with_mod_inventory(["create"]);
    assert_eq!(changed.check_errors(), Ok(()));
    assert!(old.check_errors().is_err());
}

#[test]
fn inventory_partitions_are_shared_by_readers_of_the_same_candidate() {
    let count = Arc::new(AtomicUsize::new(0));
    let original = registered(count.clone());
    let one = original.with_mod_inventory(["create"]);
    let two = original.with_mod_inventory(["create"]);
    let file = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
    assert!(one.allows_file(&file, "shared.json"));
    assert!(two.allows_file(&file, "shared.json"));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
