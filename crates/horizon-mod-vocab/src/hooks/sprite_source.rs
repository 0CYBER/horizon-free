//! The `sprite_source` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entry of an atlas definition whose `type` a mod registered (`register_sprite_sources`,
    /// vanilla's `SpriteSource.run`), asked at the entry's place in the definition's `sources`,
    /// each time the atlas is loaded or reloaded, on the resource loader. `config` is the entry's
    /// JSON object converted to a `Tag`, `atlas` the atlas being listed and `listed` the sprites
    /// the entries before it listed. Push `SpriteSourceOutput`s to `output` only for a source your
    /// mod owns; they apply in order once every mod has answered.
    #[hook(
        side = "client",
        contract = "collect",
        out = "output",
        context = "registration",
        neoforge = "none (vanilla SpriteSource.run of a type RegisterSpriteSourcesEvent registers; NeoForge adds no event for it)",
        target = "horizon_client_render::mod_sprite_sources::native_sprite_source"
    )]
    fn sprite_source();
}
