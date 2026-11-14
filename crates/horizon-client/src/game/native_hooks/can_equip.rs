pub(in crate::game) use super::living_change_target::NativeHookHost;
use super::*;

impl NativeHookHost<'_> {
    /// NeoForge's `IItemExtension.canEquip`: may `stack` go into the equipment slot `slot` (an
    /// `EquipmentSlot` name) of `entity`, of type `kind`? `can_equip` arrives holding the stack's
    /// `equippable` answer (`LivingEntity.isEquippableInSlot`) and the parts of its item may
    /// change it. `stack` is converted once and lent to each slot and entity a decision asks
    /// about (`equip_answers.rs`). Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_can_equip(
        &self,
        stack: &mod_api::ItemStack,
        slot: &str,
        entity: LivingRef,
        kind: &str,
        can_equip: &mut bool,
    ) {
        self.fire_can_equip(stack, slot, entity, kind, can_equip);
    }
}
