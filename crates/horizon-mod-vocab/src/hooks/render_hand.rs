//! The `render_hand` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0098

pub trait Hooks {
    /// Before one first-person hand of this window's player is drawn
    /// (`RenderHandEvent`, client): a hand that holds an item or a map, the
    /// empty main hand's arm, and the empty off hand, where vanilla draws
    /// nothing. Adjust `render`'s pose, draw in the hand with `draw`, or
    /// cancel the default geometry; what the mods drew stays. Equip progress
    /// includes the item's swap scale. `draw`'s pose stack starts in the
    /// hands' view space, in blocks, +x right, +y up, -z into the screen,
    /// under the hand sway and before vanilla's per-item transforms; both
    /// hands share it, and `render.hand` says which one is asked. It is lit
    /// with the light at the eyes; add geometry with `native_draw_cube`,
    /// `native_draw_quad`, `native_draw_block` and `native_draw_item`. It
    /// joins the hand's own vertices, ahead of vanilla's: the hand's field of
    /// view, the view bob, a shader pack's hand pass and `hand_drawn`
    /// see it as they see vanilla's hand. Unlike NeoForge, the pose stack is
    /// `draw`'s own: turning it moves what you draw, `render`'s pose moves
    /// the default geometry, and player render state is not exposed.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderHandEvent",
        cancellable = true
    )]
    fn render_hand();
}
