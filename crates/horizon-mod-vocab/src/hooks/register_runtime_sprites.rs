//! Client asset registration before each atlas build.

pub trait Hooks {
    /// Collect static, mod-generated PNGs for the block and GUI atlases. Called at
    /// first client load and every resource reload, before textures are stitched.
    #[notes = r#"## Runtime sprites

`register_runtime_sprites` receives a registration context and a collection of
`RuntimeSprite` declarations. Use a mod namespace and an id such as
`mymod:block/marker` for a block sprite or `mymod:gui/marker` for a GUI sprite.
Push a static PNG no larger than 256 by 256 pixels or 256 KiB. The load accepts
up to 128 sprites and one million source pixels across all mods. Existing pack
textures win and the first accepted registration of a path wins. Invalid,
duplicate and over-budget entries are logged and skipped individually.

The hook runs at client startup and on the game-owning thread before reload
staging. Re-submit
sprites on each call. Look them up with `texture_sprites` or `gui_sprite` after
`texture_atlas_stitched`, then reacquire regions after a successful reload.
A failed reload keeps the published atlases. No generated PNGs are written to disk."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "sprites",
        context = "registration",
        neoforge = "none (runtime registration before client atlas construction)"
    )]
    fn register_runtime_sprites();
}
