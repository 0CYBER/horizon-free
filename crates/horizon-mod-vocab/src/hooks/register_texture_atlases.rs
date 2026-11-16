//! Client texture atlases a mod adds before each atlas build.

pub trait Hooks {
    /// Declare atlases beside the game's (`RegisterTextureAtlasesEvent`), each with the atlas
    /// definition that lists its sprites. Called at first client load and every resource reload,
    /// before textures are stitched.
    #[notes = r#"## Texture atlases

`register_texture_atlases` receives a registration context and a collection of
`TextureAtlasRegistration` declarations: an atlas id in the mod's own namespace
(`mymod:icons`) and its definition (`mymod:icons` reads
`assets/mymod/atlases/icons.json`). The definition is a `sources` list like
vanilla's atlas definitions, and resource packs change it the same way.

Horizon packs vanilla's atlases into two, so a registered atlas's sprites are
stitched into the GUI atlas, each under `TextureAtlasRegistration::sprite_name`:
sprite `mymod:heart` of `mymod:icons` is `mymod:gui/atlases/icons/mymod/heart`.
Look it up with `gui_sprite` in `texture_atlas_stitched` for `TextureAtlas::Gui`
and draw it with `DrawContext::sprite` by that name. Every reload lists the
sprites again from the definition as the packs now give it.

The hook runs at client startup and on the game-owning thread before reload
staging; re-submit the atlases on each call. The game's own atlases never
change: atlases in the `minecraft` or `horizon` namespace are refused. The load
accepts up to 32 atlases and 4096 of their sprites across all mods. The first
registration of an atlas id wins; an unknown or malformed definition, a
duplicate atlas and sprites past the budget are logged and skipped. A pack file
already at a sprite's name wins over the sprite."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "atlases",
        context = "registration",
        neoforge = "RegisterTextureAtlasesEvent"
    )]
    fn register_texture_atlases();
}
