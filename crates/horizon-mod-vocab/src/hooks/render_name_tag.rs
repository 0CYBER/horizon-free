//! The `render_name_tag` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Name tag visibility and optional replacement text.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderNameTagEvent",
        cancellable = false
    )]
    fn render_name_tag();
}
