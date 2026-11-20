use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `IItemExtension.getEquipSound`: the sound event `stack` plays when it
    /// is equipped in `slot`, starting from the stack's own `equippable`
    /// component. The parts of `stack`'s item may change it. Server-side.
    #[inline(never)]
    pub(in crate::game) fn native_get_equip_sound(
        &self,
        stack: &ItemStack,
        slot: &str,
        sound: &mut Option<Identifier>,
    ) {
        self.fire_get_equip_sound(stack, slot, sound);
    }

    /// The short equipment slot names, as `native_on_equip` hears them, for a
    /// `LivingRef` change.
    pub(in crate::game) fn equip_slot_name(&self, slot: mod_api::EquipmentSlot) -> &'static str {
        match slot {
            mod_api::EquipmentSlot::MainHand => "mainhand",
            mod_api::EquipmentSlot::OffHand => "offhand",
            mod_api::EquipmentSlot::Feet => "feet",
            mod_api::EquipmentSlot::Legs => "legs",
            mod_api::EquipmentSlot::Chest => "chest",
            mod_api::EquipmentSlot::Head => "head",
            mod_api::EquipmentSlot::Body => "body",
            mod_api::EquipmentSlot::Saddle => "saddle",
        }
    }

    /// The equip sound for `stack` worn in `slot`, starting from `own` (the
    /// stack's own `equippable` sound): the item's parts get to change it
    /// through `get_equip_sound`. The game keeps string sound ids, so the API's
    /// `Identifier` is parsed and written back here. `None` keeps the generic
    /// armour sound the caller falls back to.
    pub(in crate::game) fn mod_equip_sound(
        &self,
        stack: &ItemStack,
        slot: &str,
        own: Option<String>,
    ) -> Option<String> {
        let mut sound = own.and_then(|s| Identifier::parse(&s).ok());
        self.native_get_equip_sound(stack, slot, &mut sound);
        sound.map(|sound| sound.to_string())
    }
}
