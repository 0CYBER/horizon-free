//! Player hand access through the selected inventory slot and shared equipment.
//! Equipping writes storage before invoking LivingEntity's onEquipItem hook.
use crate::{
    item_stack_refs::StackRef,
    player_inventory::{EquipmentSlot, PlayerInventory},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionHand {
    Main,
    Off,
}
impl InteractionHand {
    pub fn slot(self) -> EquipmentSlot {
        match self {
            Self::Main => EquipmentSlot::MainHand,
            Self::Off => EquipmentSlot::OffHand,
        }
    }
}
pub fn get<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq>(
    inv: &PlayerInventory<I, K, V>,
    slot: EquipmentSlot,
) -> StackRef<I, K, V> {
    if slot == EquipmentSlot::MainHand {
        inv.selected_item()
    } else {
        inv.equipment().get(slot)
    }
}
pub fn set<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq>(
    inv: &mut PlayerInventory<I, K, V>,
    slot: EquipmentSlot,
    stack: StackRef<I, K, V>,
) -> StackRef<I, K, V> {
    if slot == EquipmentSlot::MainHand {
        inv.set_selected_item(stack)
    } else {
        inv.equipment().set(slot, stack)
    }
}
pub fn is_empty<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq>(
    inv: &PlayerInventory<I, K, V>,
) -> bool {
    inv.selected_item().read().is_empty()
        && inv
            .equipment()
            .entries()
            .iter()
            .all(|(_, s)| s.read().is_empty())
}
pub trait EquipWorld<I, K, V> {
    type Error;
    fn on_equip(
        &mut self,
        inv: &mut PlayerInventory<I, K, V>,
        slot: EquipmentSlot,
        old: StackRef<I, K, V>,
        new: StackRef<I, K, V>,
    ) -> Result<(), Self::Error>;
}
pub fn set_item_slot<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, W: EquipWorld<I, K, V>>(
    w: &mut W,
    inv: &mut PlayerInventory<I, K, V>,
    slot: EquipmentSlot,
    stack: StackRef<I, K, V>,
) -> Result<(), W::Error> {
    let old = set(inv, slot, stack.clone());
    w.on_equip(inv, slot, old, stack)
}
pub fn get_hand<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq>(
    inv: &PlayerInventory<I, K, V>,
    hand: InteractionHand,
) -> StackRef<I, K, V> {
    get(inv, hand.slot())
}
pub fn set_hand<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, W: EquipWorld<I, K, V>>(
    w: &mut W,
    inv: &mut PlayerInventory<I, K, V>,
    hand: InteractionHand,
    stack: StackRef<I, K, V>,
) -> Result<(), W::Error> {
    set_item_slot(w, inv, hand.slot(), stack)
}
