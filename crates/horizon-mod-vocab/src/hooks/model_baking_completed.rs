//! The `model_baking_completed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Every model of a resource load is baked and published (`ModelEvent.BakingCompleted`): the
    /// block and item models and the standalone ones mods asked for (`standalone_models`), in the
    /// store `model` now reads. Fires once per successful model load, after both
    /// `texture_atlas_stitched` calls: on the first load before `client_setup`, and after every
    /// successful resource reload (F3+T, a pack turned on or off, a server's pack) before
    /// `client_resources_reloaded`; a reload that fails fires nothing, and the old handles stay
    /// right. Look the models a mod draws up here, once, through `render_assets().model`, and
    /// keep the `ModelRef`s until the next `model_baking_completed`, when the old ones answer
    /// `ModelError::StaleModel`. Never look them up by name every frame. Client-side.
    #[notes = r#"## Model baking completed

`model_baking_completed` (NeoForge's `ModelEvent.BakingCompleted`) fires on the render thread once per successful model
load, right after both atlases' `texture_atlas_stitched`: on the first load before `client_setup`, and after every
successful resource reload before `client_resources_reloaded`. The store `render_assets().model(&id)` reads is already
the new one, the standalone models included. Resolve the models a mod draws there, once, and keep the `ModelRef`s until
the next `model_baking_completed`, when the old ones answer `ModelError::StaleModel`; a reload that fails fires nothing
and leaves them valid. It gives no view of the baked geometry and edits nothing (that is #861). A dedicated server never
fires it."#]
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "ModelEvent.BakingCompleted"
    )]
    fn model_baking_completed();
}
