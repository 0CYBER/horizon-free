//! The `camera_submersion` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Camera submersion fog type (`Camera.getFluidInCamera`), checked before fog and camera
    /// fluid effects are set up. Allows native mods (such as Freecam) to clear underwater,
    /// lava or powder snow submersion by setting `fog_type` to `FogType::Atmospheric`.
    #[hook(side = "client", contract = "listen")]
    fn camera_submersion();
}
