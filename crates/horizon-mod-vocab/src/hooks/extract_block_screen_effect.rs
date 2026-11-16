//! The `extract_block_screen_effect` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before one of the first-person view's block overlays (`ScreenEffectRenderer`) is drawn,
    /// once a frame each is shown: `Block` with the view-blocking `state` at `pos` the camera is
    /// inside, `Water` with the `state` at the eye's block `pos` while the eye is under water, and
    /// `Fire` with fire's default `state` at the player's block `pos` while they burn. Never for a
    /// spectator; `Fire` only in first person. Set `canceled` to leave that overlay out; the
    /// frame, the other block overlays and the HUD's camera overlays are drawn as before. Set
    /// `custom_draw` to draw it yourself (`block_screen_effect_draw`), canceled or not.
    /// `partial_tick` is the world's.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ExtractBlockScreenEffectEvent",
        cancellable = true
    )]
    #[notes = r##"## Block screen effects

`extract_block_screen_effect(effect, state, pos, partial_tick, canceled, custom_draw)` is NeoForge's `ExtractBlockScreenEffectEvent`: once a frame for each of the first-person view's block overlays that is shown, before its quads are built. `Block` is the view-blocking block the camera is inside, `Water` the block at the eye while it is under water, and `Fire` fire's default state at the player's block while they burn (first person only). None of them fire for a spectator. Set `canceled` to leave that one overlay out; the other overlays and the HUD's camera overlays (vignette, pumpkin, portal, freezing) stay.

Set `custom_draw`, NeoForge's `setCustomRenderer`, to draw the overlay yourself: `block_screen_effect_draw` is then asked about it later in the frame. A mod can't hand over a renderer object, so the flag stands in for the renderer and the hook for its `submit`. Vanilla's overlay still draws after the mod's, unless the draw hook sets `replace_vanilla` or the overlay was canceled. A canceled overlay with `custom_draw` set still gets the draw hook, and returning `Flow::Cancel` keeps a `custom_draw` set on the event; canceling without it is `CustomBlockScreenEffectRenderer.NO_OP`. Canceling skips later mods, so they neither see the overlay nor set `custom_draw` on it. The player and the camera are on `ClientHost` (`local_player`, `camera`, `camera_entity`, `camera_type`), and `partial_tick` is `getWorldPartialTick`, the same value the draw hook gets. `getPlayerPartialTick` is not provided. With no listener the frame pays one `listened` check."##]
    fn extract_block_screen_effect();
}
