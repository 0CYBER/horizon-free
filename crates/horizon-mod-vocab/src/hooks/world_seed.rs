//! Upstream capability retained through its existing fork handle.
pub trait Notes {
    #[notes = r###"`native_world_seed()`, the world's seed as `/seed` prints it (Bukkit's `World.getSeed`), the one every level generates from; None off a server. Bundles need "hooks": @API:world_seed@."###]
    fn world_seed();
}
