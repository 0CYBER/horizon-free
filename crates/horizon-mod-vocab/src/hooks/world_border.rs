//! Border reads carried from upstream through a level handle.
pub trait Notes {
    #[notes = r#"`native_world_border(dimension)`, the bounds of a level's world border as the server enforces it (one border for every level in Horizon today). Bundles need "hooks": @API:world_border@."#]
    fn world_border();
}
