use super::*;

impl Game<'_> {
    /// Client-side value of a categorical item model property a mod registered
    /// (`register_select_item_model_property`), asked each time a `minecraft:select` on it picks a
    /// case. Set value only for a property your mod owns; left `None`, the model's fallback is drawn.
    /// The owned stack snapshot is the one being resolved, including nested bundle items.
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    pub(in crate::game) fn native_item_model_select_property(
        &self,
        property: &str,
        stack: &mod_api::ItemStack,
        bundle_selected: i32,
        value: &mut Option<String>,
    ) {
        self.fire_item_model_select_property(property, stack, bundle_selected, value);
    }
}
