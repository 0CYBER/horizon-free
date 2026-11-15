#[test]
pub(crate) fn tagged_dispatch_requires_its_own_world_table() {
    game_thread();
    resolve(&["test:gauge"]).unwrap();
    let scope = crate::testing::inherit_mods(move |_: &()| {
        assert!(
            mod_vocab::__private::parts_installation::check_parts(
                HookId::ShouldCheckWeakPower,
                "other"
            )
            .is_ok()
        );
        let error = mod_vocab::__private::parts_installation::check_parts(
            HookId::AnalogOutput,
            "analog_output",
        )
        .err()
        .unwrap();
        let panic = std::panic::catch_unwind(|| read(1)).unwrap_err();
        (error, panic)
    });
    let failure = std::thread::Builder::new()
        .name("part-worker".into())
        .spawn(move || scope(&()))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(failure.0.hook, "analog_output");
    assert_eq!(failure.0.thread, "part-worker");
    let message = failure.1.downcast_ref::<String>().unwrap();
    assert!(message.contains("analog_output") && message.contains("part-worker"));
    assert_eq!(read(1), 106);
    super::clear_game_parts();
    assert_eq!(read(1), 1);
    resolve(&["test:gauge"]).unwrap();
    assert_eq!(read(1), 106);
    mark_game_thread();
    assert_eq!(read(1), 1);
}
