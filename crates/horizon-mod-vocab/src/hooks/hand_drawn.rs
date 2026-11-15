//! The `hand_drawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The first-person hand is drawn (see [`HandDrawn`]), on every frame [`world_drawn`]
    /// runs, with no hand too.
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::hand_drawn",
        neoforge = "RenderHandEvent"
    )]
    fn hand_drawn();
}
