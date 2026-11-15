//! The `render_level_stage` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Draw world geometry at one stage of a world view (NeoForge 26.3's
    /// `RenderLevelStageEvent`), asked for each `RenderLevelStage` in frame order. `draw` is
    /// the view's `WorldDrawContext` and shares its limits with `submit_custom_geometry`; its
    /// lines and quads draw at `stage`. Client-only, noncancellable.
    #[notes = r#"## World render stages

`Hooks::render_level_stage` is the client counterpart of NeoForge 26.3's
`RenderLevelStageEvent` for the stages Horizon draws, as one hook with a `RenderLevelStage`:
`AfterSky` (behind everything else, for backdrops and atmosphere), `AfterOpaqueBlocks` (after
solid and cutout terrain, before entities), `AfterTranslucentBlocks` (after water and stained
glass, before translucent particles), `AfterWeather` (after rain and snow, before the world
border) and `AfterLevel` (with `Hooks::submit_custom_geometry`, before the hand). It is asked
for each stage in that order, every view of a world, and never on title or connect screens.

Horizon gathers a view's geometry before rendering it, so every stage is asked as the view is
extracted, in stage order, with `Hooks::submit_custom_geometry` and the debug renderers asked
just before `AfterLevel`. All take the same `WorldRenderView` and `WorldDrawContext`:
[the custom geometry limits and rules](#custom-world-geometry) apply and are shared by all stages. Only where it is drawn differs. `draw_model_instances` copies
are part of the world's model pass, so they draw with entities whatever the stage.

A route that does not draw a stage's part of the world in the world pass draws that stage's
geometry at the next stage it reaches instead, depth tested as usual: a shader pack that draws
the sky, terrain, water or weather itself, and improved transparency for the translucent and
weather stages. Nothing is dropped, and the order between stages is kept."#]
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderLevelStageEvent",
        cancellable = false
    )]
    fn render_level_stage();
}
