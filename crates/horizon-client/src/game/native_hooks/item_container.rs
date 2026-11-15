use super::*;

/// The registered item-container provider's runtime slot count.
impl Game<'_> {
    /// Item `item` holds items (`Capabilities.Item.ITEM`) when an explicit
    /// per-item registration supplies slots for it. Clamped to 1..=256;
    /// 0 leaves the item without slots, and vanilla's items have none. Asked at each
    /// call of the item container helpers (`item_container_slots`,
    /// `item_container_insert`, `item_container_extract`, in
    /// `native_item_container.rs`). The contents are kept in the stack's
    /// `minecraft:container`, as a shulker box item keeps them.
    /// API 370.
    pub(in crate::game) fn native_item_container(&self, item: &str, slots: &mut Option<u32>) {
        *slots = mod_api::__private::parts_installation::item_container_slots(self, item);
    }
}
