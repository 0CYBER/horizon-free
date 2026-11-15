//! The `render_item_in_frame` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a visible item frame's item or map is drawn. Cancel hides the item, not the frame.
    #[notes = r#"## Item frame rendering

`render_item_in_frame(entity_id, pos, stack, canceled)` runs before a visible item frame draws its item or map, including glow and invisible frames. Set `canceled` to hide the item without hiding the frame."#]
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderItemInFrameEvent",
        cancellable = true
    )]
    fn render_item_in_frame();
}
