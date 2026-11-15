//! The `item_model_select_property` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Client-side value of a categorical item model property a mod registered
    /// (`register_select_item_model_property`), asked each time a `minecraft:select` on it picks a
    /// case. Set value only for a property your mod owns; left `None`, the model's fallback is drawn.
    /// The owned stack snapshot is the one being resolved, including nested bundle items.
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RegisterSelectItemModelPropertyEvent"
    )]
    fn item_model_select_property();
}
