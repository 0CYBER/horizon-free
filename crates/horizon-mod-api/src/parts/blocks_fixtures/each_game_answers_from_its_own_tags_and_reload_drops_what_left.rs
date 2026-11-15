#[test]
pub(crate) fn each_game_answers_from_its_own_tags_and_reload_drops_what_left() {
    let mods = game_thread();
    // Both games resolve before either reads, so one shared table would fail one of them.
    let resolved = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let client = scope.spawn(|| {
            crate::__private::install(mods.clone());
            install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
            mark_game_thread();
            resolve(&["test:other"]).unwrap();
            resolved.wait();
            (read(1), read(2))
        });
        let server = scope.spawn(|| {
            crate::__private::install(mods.clone());
            install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
            mark_game_thread();
            resolve(&["test:gauge"]).unwrap();
            resolved.wait();
            let before = (read(1), read(2));
            // `/reload` with the tag emptied.
            resolve(&[]).unwrap();
            (before, read(1))
        });
        // The tag part comes after the key part of the same mod: 1 + 5 + 100.
        assert_eq!(client.join().unwrap(), (6, 101));
        assert_eq!(server.join().unwrap(), ((106, 1), 6));
    });
}
