//! The `modify_default_components` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0092

pub trait Hooks {
    /// NeoForge 26.3's ModifyDefaultComponentsEvent: set or remove entries in
    /// `components`, the default component map for `item`, including vanilla items.
    /// Asked once per item after registries freeze, before the owning load
    /// publishes immutable defaults. Receives a read-only DataLoadContext with
    /// the actual ServerDataLoad or ClientPreload origin, frozen registry names,
    /// settings captured once for loaded packages at this load boundary, and
    /// the shared LoadedMods view. Readers and datapack reloads do not replay
    /// this event. No runtime Host, world handles or transaction access.
    #[hook(
        side = "both",
        contract = "collect",
        out = "components",
        context = "data_load",
        version = 2,
        target = "horizon_world::item_defaults::native_modify_default_components",
        neoforge = "ModifyDefaultComponentsEvent"
    )]
    fn modify_default_components();
}
