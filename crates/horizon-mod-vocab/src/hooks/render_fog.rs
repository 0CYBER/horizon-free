//! The `render_fog` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0119

pub trait Hooks {
    /// Final world fog, after the computed color hook, ordinary calculations and Horizon's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.RenderFog"
    )]
    fn render_fog();
}
