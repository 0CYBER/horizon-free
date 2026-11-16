/// The test mods, their parts installed, on a thread that runs a game.
pub fn game_thread() -> crate::Mods {
    let mods: crate::Mods = vec![
        ModEntry::new("doubler", Box::new(Doubler)),
        ModEntry::new("chemist", Box::new(Chemist)),
        ModEntry::new("test", Box::new(FixtureContent)),
    ]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mod_vocab::__private::resolve_fluids(
        &[
            "minecraft:water",
            "minecraft:lava",
            "minecraft:flowing_water",
            "minecraft:flowing_lava",
            "minecraft:empty",
        ],
        &[],
    )
    .unwrap();
    mod_vocab::__private::resolve_block_keys(
        [
            "minecraft:air",
            "minecraft:granite",
            "test:gauge",
            "test:other",
            "test:connector",
            "test:press",
            "test:void_bin",
            "test:ordered",
            "minecraft:stone",
        ],
        |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|e| format!("{e:?}"))
        },
    )
    .unwrap();
    mark_game_thread();
    mods
}
