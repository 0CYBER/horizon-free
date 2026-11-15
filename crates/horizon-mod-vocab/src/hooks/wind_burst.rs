pub trait Hooks {
    /// Before a wind burst interacts with blocks, `wind_burst(dimension, center, blocks)` receives the shuffled candidate positions. Remove positions to spare those blocks; entity knockback and the shuffle's random draws are unchanged. Not called when mob_griefing prevents a breeze from interacting with blocks. Bundles need "hooks": @API:wind_burst@.
    #[hook(
        side = "server",
        contract = "collect",
        out = "blocks",
        neoforge = "none (vanilla wind-charge block interaction; NeoForge adds no event)"
    )]
    fn wind_burst();
}
