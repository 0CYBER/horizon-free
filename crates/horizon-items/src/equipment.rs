//! `EntityEquipment`: the stack in each `EquipmentSlot` of a living entity,
//! and its `equipment` compound (`EntityEquipment.CODEC`). A mob keeps all
//! eight slots here; the host player and LAN guests keep body and saddle,
//! and all of them save through it.
use crate::item_nbt::{item_from_nbt, item_to_nbt};
use crate::items::ItemStack;
use horizon_core::nbt::{Compound, Tag};

/// The slot names of the compound, by `EquipmentSlot` ordinal.
pub const SLOT_NAMES: [&str; 8] = [
    "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
];
/// The ordinals of `EquipmentSlot`.
pub const MAINHAND: usize = 0;
pub const OFFHAND: usize = 1;
pub const FEET: usize = 2;
pub const LEGS: usize = 3;
pub const CHEST: usize = 4;
pub const HEAD: usize = 5;
pub const BODY: usize = 6;
pub const SADDLE: usize = 7;
/// The armour slots, head to feet.
pub const ARMOR: [usize; 4] = [HEAD, CHEST, LEGS, FEET];

/// A stack for each `EquipmentSlot`, indexed by ordinal. A player holder keeps
/// only body and saddle here: its hands, armor and inventory live in the
/// player's own fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equipment([Option<ItemStack>; 8]);

impl Equipment {
    pub fn get(&self, slot: usize) -> Option<&ItemStack> {
        self.0.get(slot)?.as_ref()
    }
    /// Puts `stack` in `slot` (`EntityEquipment.set`); false for a slot out
    /// of range.
    pub fn set(&mut self, slot: usize, stack: Option<ItemStack>) -> bool {
        match self.0.get_mut(slot) {
            Some(cell) => {
                *cell = stack;
                true
            }
            None => false,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }
    /// Empties every slot and returns what they held, in slot order.
    pub fn take_all(&mut self) -> impl Iterator<Item = ItemStack> + '_ {
        self.0.iter_mut().filter_map(Option::take)
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Option<ItemStack>> {
        self.0.iter()
    }
    /// `EntityEquipment.CODEC` read: the slots of an `equipment` compound.
    pub fn read(tag: &Tag) -> Self {
        let mut out = Self::default();
        for (cell, name) in out.0.iter_mut().zip(SLOT_NAMES) {
            *cell = tag.get(name).and_then(item_from_nbt);
        }
        out
    }
    /// `EntityEquipment.CODEC` write: None when nothing is equipped, as
    /// `LivingEntity.addAdditionalSaveData` omits the key then.
    pub fn write(&self) -> Option<Tag> {
        let mut compound = Compound::new();
        for (slot, stack) in self.0.iter().enumerate() {
            put(&mut compound, slot, stack.as_ref());
        }
        (!compound.is_empty()).then_some(Tag::Compound(compound))
    }
    /// Sets the `equipment` key of an entity's save compound from `write`,
    /// removing a stale one when nothing is equipped.
    pub fn store(&self, entity: &mut Compound) {
        match self.write() {
            Some(tag) => {
                entity.insert("equipment".into(), tag);
            }
            None => {
                entity.remove(&"equipment".into());
            }
        }
    }
}
impl std::ops::Index<usize> for Equipment {
    type Output = Option<ItemStack>;
    fn index(&self, slot: usize) -> &Self::Output {
        &self.0[slot]
    }
}
impl std::ops::IndexMut<usize> for Equipment {
    fn index_mut(&mut self, slot: usize) -> &mut Self::Output {
        &mut self.0[slot]
    }
}
impl IntoIterator for Equipment {
    type Item = Option<ItemStack>;
    type IntoIter = std::array::IntoIter<Option<ItemStack>, 8>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl From<[Option<ItemStack>; 8]> for Equipment {
    fn from(slots: [Option<ItemStack>; 8]) -> Self {
        Self(slots)
    }
}

/// Sets or clears one slot of an `equipment` compound.
fn put(compound: &mut Compound, slot: usize, stack: Option<&ItemStack>) {
    let name = SLOT_NAMES[slot];
    match stack {
        Some(stack) => {
            compound.insert(name.into(), item_to_nbt(stack));
        }
        None => {
            compound.remove(&name.into());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compound_round_trips_and_omits_empty_slots() {
        let mut e = Equipment::default();
        assert!(e.write().is_none());
        let mut saddle = ItemStack::new("minecraft:saddle", 1);
        saddle.damage = 2;
        assert!(e.set(SADDLE, Some(saddle)));
        assert!(e.set(BODY, Some(ItemStack::new("minecraft:iron_horse_armor", 1))));
        assert!(!e.set(8, None));
        let tag = e.write().unwrap();
        assert!(tag.get("head").is_none() && tag.get("saddle").is_some());
        assert_eq!(Equipment::read(&tag), e);
        assert_eq!(e.take_all().count(), 2);
        assert!(e.is_empty());
    }
}
