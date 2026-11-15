//! The `block_screen_effect_draw` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Draw a block overlay a mod said it draws itself (`custom_draw` in
    /// `extract_block_screen_effect`), before vanilla's overlay of that kind, once a frame each.
    /// A canceled overlay is asked too. Set `replace_vanilla` to leave vanilla's overlay out.
    /// `effect`, `state` and `pos` are the extract event's; `partial_tick` is the world's and
    /// `hide_gui` says the HUD is hidden. Every mod is asked about every such overlay, so a mod
    /// ignores those it did not claim. Client-side.
    #[notes = r#"## Block screen effect drawing

`Hooks::block_screen_effect_draw` is `submit` of NeoForge 26.3's `CustomBlockScreenEffectRenderer`, set through `custom_draw` in `extract_block_screen_effect`, and `replace_vanilla` is what `submit` returns: true prevents vanilla rendering. It runs in the frame's `ScreenEffectRenderer` step, after every overlay's extract event, in vanilla's order: block, water, fire. What it draws for an overlay goes right before that overlay and after the overlays before it, and vanilla's camera overlays (vignette, pumpkin, portal, freezing) go over all of them.

`draw` is a `CameraDrawContext`, not the HUD's `DrawContext`: vanilla's screen effects are camera-space quads under the first-person hand's projection (`GameRenderer.render3dHud`), in the pass after the level and the hand and before the GUI. Vanilla's own overlay covers the view with corners at `(±1, ±1, -0.5)`. `submit`'s `PoseStack` starts as the identity there, so a corner given to `draw` is what a corner under that fresh pose would be: there is no pose stack, and a mod moves its corners itself. `submit`'s sprites are the names `CameraDrawContext::sprite` takes. Its player and camera render states are `ClientHost`'s player and camera reads. Nothing a mod draws outlives the frame, and no GPU handle is exposed.

With no `custom_draw` set the hook is never asked, so a mod that only listens here draws nothing."#]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "CustomBlockScreenEffectRenderer",
        cancellable = false
    )]
    fn block_screen_effect_draw(
        effect: mod_api::BlockScreenEffect,
        state: BlockState,
        pos: BlockPos,
        partial_tick: f32,
        hide_gui: bool,
        draw: &mut dyn CameraDrawContext,
        replace_vanilla: &mut bool,
    );
}
