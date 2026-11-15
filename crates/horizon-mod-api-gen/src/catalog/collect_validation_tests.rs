use super::Hook;
use serde_json::json;

#[test]
fn validators_are_paths_and_only_apply_to_global_collectors() {
    let parse = |api| {
        Hook::parse(json!({
            "name": "validated", "doc": "Doc.", "side": "both", "target": "crate::native_validated",
            "mod_api": api,
        }))
    };
    assert_eq!(
        parse(json!({"contract":"collect", "out":"items", "collect_validator":"crate::validate"}))
            .unwrap()
            .collect_validator
            .as_deref(),
        Some("crate::validate")
    );
    for api in [
        json!({"contract":"listen", "collect_validator":"crate::validate"}),
        json!({"contract":"collect", "out":"items", "scope":"item", "key":"stack", "collect_validator":"crate::validate"}),
        json!({"contract":"collect", "out":"items", "collect_validator":"not a path; panic!()"}),
        json!({"contract":"collect", "out":"items", "answered_by":"mod", "collect_validator":"crate::validate"}),
    ] {
        assert!(parse(api).is_err());
    }
}
