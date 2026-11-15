//! The `tooltip_component` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0076

pub trait Hooks {
    /// Client-side image component for this stack's tooltip. Answer its width/height
    /// in GUI pixels; finite positive dimensions reserve space after text and any
    /// vanilla bundle component. None keeps the vanilla tooltip unchanged.
    #[hook(
        side = "client",
        contract = "replace",
        out = "size",
        scope = "item",
        key = "stack",
        neoforge = "RegisterClientTooltipComponentFactoriesEvent"
    )]
    fn tooltip_component();
}
