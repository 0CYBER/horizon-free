#[test]
pub(crate) fn a_key_the_game_lacks_fails_naming_the_mod() {
    game_thread();
    let refused = resolve_parts(
        &HashMap::new(),
        &HashMap::new(),
        KnownIds {
            block: &|id| id != "test:gauge",
            item: &|_| true,
        },
    )
    .unwrap_err();
    let super::ResolvePartsError::Parts(refused) = refused else {
        panic!("wrong refusal")
    };
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].starts_with("doubler attached "));
    assert!(refused[0].contains("Gauge to block test:gauge, which this game does not have"));
}
