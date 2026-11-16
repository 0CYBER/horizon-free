/// Install fixture parts with the test rule schema.
pub fn install_parts_fixture(mods: &[ModEntry]) -> Result<(), Vec<String>> {
    // This API-only fixture has no engine dimension codec. Dimension integration tests
    // use the game's installer, which supplies its real shape validator.
    install_parts(mods, &rule_schema(), |_| false)
}
