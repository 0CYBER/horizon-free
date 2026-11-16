#[test]
fn prepared_key_routing_tracks_tag_reload() {
    game_thread();
    mod_vocab::__private::resolve_block_keys(
        [
            "minecraft:air",
            "test:gauge",
            "test:other",
            "test:connector",
            "test:press",
            "minecraft:granite",
            "test:void_bin",
            "test:ordered",
        ],
        |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|e| format!("{e:?}"))
        },
    )
    .unwrap();
    let other = mod_vocab::__private::block_key("test:other").unwrap();
    resolve(&["test:other"]).unwrap();
    assert!(block_key_listens(HookId::AnalogOutput, other));
    resolve(&[]).unwrap();
    assert!(!block_key_listens(HookId::AnalogOutput, other));
    assert!(has_scoped_hook(HookId::AnalogOutput));
    assert!(resolve(&["missing:block"]).is_err());
    assert!(!block_key_listens(HookId::AnalogOutput, other));
    assert_eq!(
        read(1),
        6,
        "a refused preparation preserves the published Parts"
    );
}
