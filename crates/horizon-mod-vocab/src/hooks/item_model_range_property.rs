//! The `item_model_range_property` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Client-side value of a numeric item model property a mod registered
    /// (`register_range_select_item_model_property`), asked each time a `minecraft:range_dispatch`
    /// on it picks a branch. Set value only for a property your mod owns; unanswered it stays 0.
    /// The model's `scale` is applied afterwards. The owned stack snapshot is the one being resolved,
    /// including nested bundle items.
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RegisterRangeSelectItemModelPropertyEvent"
    )]
    fn item_model_range_property();
}
