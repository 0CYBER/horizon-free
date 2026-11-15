//! The `item_stacked_on_other` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override. It runs inside a pickup click. Answer from the arguments and `context` alone, and fast: `context` holds the side, the registries and the settings of the resource generation, and a `/reload` builds the next one.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        context = "loop",
        neoforge = "ItemStackedOnOtherEvent",
        cancellable = true,
        version = 2,
        target = "horizon_client_lib::menu::native_item_stacked_on_other"
    )]
    fn item_stacked_on_other();
}
