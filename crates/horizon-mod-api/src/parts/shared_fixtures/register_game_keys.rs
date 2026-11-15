/// Register the game fixture's final block registry through the actual registrar, retaining
/// its installed test mods and Parts. Production mods request only the entries they use.
pub fn register_game_keys(
    ids: impl IntoIterator<Item = String>,
    schema: &mod_vocab::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) {
    mod_vocab::testing::rebuild_parts(|| {
        let mut registrar = super::register_all(crate::__private::mods(), dimension_shape);
        registrar.mod_id = "horizon";
        registrar.declare_fluid("definition_test");
        for id in ids {
            registrar.block_key(&id);
        }
        super::install_registrar(registrar, schema)?;
        mod_vocab::__private::resolve_fluids(
            &[
                "minecraft:empty",
                "minecraft:water",
                "minecraft:flowing_water",
                "minecraft:lava",
                "minecraft:flowing_lava",
            ],
            // The engine's world fluids in vanilla registry order, as the game resolves them.
            &[
                "minecraft:empty",
                "minecraft:flowing_water",
                "minecraft:water",
                "minecraft:flowing_lava",
                "minecraft:lava",
            ],
        )
        .unwrap();
        Ok::<(), Vec<String>>(())
    })
    .unwrap();
}
