use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.getCreatorModId`: the mod that made `stack`.
    /// `mod_id` arrives as the item id's namespace; set it to another mod,
    /// or `None`.
    pub(in crate::game) fn native_creator_mod_id(
        &self,
        stack: &ItemStack,
        mod_id: &mut Option<String>,
    ) {
        self.fire_creator_mod_id(stack, mod_id);
    }
}
