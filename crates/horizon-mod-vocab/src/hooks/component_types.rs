//! The `component_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0074

pub trait Hooks {
    /// Register namespaced NBT component types before content loads.
    #[hook(
        side = "both",
        contract = "collect",
        out = "types",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn component_types();
}
