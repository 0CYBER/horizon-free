//! Read-only equipment host: cached target facts, ordinary game reads elsewhere.
use super::Game;
use mod_api::__private::game::BackendRefOf;
use mod_api::{Backend, BackendRef, EquipReadRow, EquipmentSlot, Host, ItemStack};
pub(in crate::game) struct EquipHost<'g, 'a> {
    pub(in crate::game) game: &'g Game<'a>,
    pub(in crate::game) row: &'g EquipReadRow,
}
impl EquipHost<'_, '_> {
    pub(in crate::game) fn equipment(&self, slot: EquipmentSlot) -> Option<ItemStack> {
        let index = match slot {
            EquipmentSlot::MainHand => 0,
            EquipmentSlot::OffHand => 1,
            EquipmentSlot::Feet => 2,
            EquipmentSlot::Legs => 3,
            EquipmentSlot::Chest => 4,
            EquipmentSlot::Head => 5,
            EquipmentSlot::Body => 6,
            EquipmentSlot::Saddle => 7,
        };
        self.row.equipment[index].clone()
    }
}
impl Host for EquipHost<'_, '_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl Backend for EquipHost<'_, '_> {}
