//! The `render_arm` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before this window's player's first-person `left` or right arm is
    /// drawn (`RenderArmEvent`, in `renderPlayerArm` and `renderMapHand`):
    /// the empty main hand's arm, and the arms under a held map, never while
    /// the player is invisible. `player` and `player_entity_id` are the
    /// local player's UUID and entity id. `draw`'s pose stack starts at the
    /// arm model's origin, in blocks, posed as vanilla's arm is (with any
    /// `render_hand`), lit with the light at the eyes; add
    /// geometry with `native_draw_cube`, `native_draw_quad`, `native_draw_block`
    /// and `native_draw_item`. Set `canceled` to leave out vanilla's arm;
    /// what the mods drew stays. Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderArmEvent",
        cancellable = true
    )]
    fn render_arm();
}
