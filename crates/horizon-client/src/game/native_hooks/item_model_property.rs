use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Client-side evaluation of a mod's namespaced conditional item model property.
    /// Set value only for a property your mod owns; unhandled properties stay false.
    /// The owned stack snapshot is the one being resolved, including nested bundle items.
    /// `bundle_selected` is the runtime bundle selection index; -1 means none.
    pub(in crate::game) fn native_item_model_property(
        &self,
        property: &str,
        stack: &mod_api::ItemStack,
        bundle_selected: i32,
        value: &mut bool,
    ) {
        self.fire_item_model_property(property, stack, bundle_selected, value);
    }
}
