//! Player inventory storage and selection. Equipment is shared with its entity;
//! stack references preserve aliases across inventories and external callers.
//! Pickup, ticking, menus and persistence are implemented separately.
use crate::item_stack_refs::{ItemStacks, StackRef};
use std::{cell::RefCell, rc::Rc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum EquipmentSlot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}
impl EquipmentSlot {
    pub const ALL: [Self; 8] = [
        Self::MainHand,
        Self::OffHand,
        Self::Feet,
        Self::Legs,
        Self::Chest,
        Self::Head,
        Self::Body,
        Self::Saddle,
    ];
    pub fn inventory_slot(self) -> Option<usize> {
        match self {
            Self::MainHand => None,
            Self::OffHand => Some(40),
            Self::Feet => Some(36),
            Self::Legs => Some(37),
            Self::Chest => Some(38),
            Self::Head => Some(39),
            Self::Body => Some(41),
            Self::Saddle => Some(42),
        }
    }
    pub fn from_inventory_slot(index: i32) -> Option<Self> {
        match index {
            36 => Some(Self::Feet),
            37 => Some(Self::Legs),
            38 => Some(Self::Chest),
            39 => Some(Self::Head),
            40 => Some(Self::OffHand),
            41 => Some(Self::Body),
            42 => Some(Self::Saddle),
            _ => None,
        }
    }
}
type EquipmentItems<I, K, V> = [Option<StackRef<I, K, V>>; 8];
#[derive(Debug)]
pub struct EntityEquipment<I, K, V> {
    items: Rc<RefCell<EquipmentItems<I, K, V>>>,
    empty: StackRef<I, K, V>,
}
impl<I, K, V> Clone for EntityEquipment<I, K, V> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            empty: self.empty.clone(),
        }
    }
}
impl<I, K, V> EntityEquipment<I, K, V> {
    pub fn new(empty: StackRef<I, K, V>) -> Self {
        Self {
            items: Rc::new(RefCell::new(std::array::from_fn(|_| None))),
            empty,
        }
    }
    pub fn get(&self, slot: EquipmentSlot) -> StackRef<I, K, V> {
        self.items.borrow()[slot as usize]
            .clone()
            .unwrap_or_else(|| self.empty.clone())
    }
    pub fn set(&self, slot: EquipmentSlot, stack: StackRef<I, K, V>) -> StackRef<I, K, V> {
        self.items.borrow_mut()[slot as usize]
            .replace(stack)
            .unwrap_or_else(|| self.empty.clone())
    }
    /// Native clear preserves which enum-map entries have been populated.
    pub fn clear(&self) {
        for s in self.items.borrow_mut().iter_mut().flatten() {
            *s = self.empty.clone();
        }
    }
    pub fn entries(&self) -> Vec<(EquipmentSlot, StackRef<I, K, V>)> {
        let items = self.items.borrow();
        EquipmentSlot::ALL
            .into_iter()
            .filter_map(|s| items[s as usize].clone().map(|v| (s, v)))
            .collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryError {
    Index,
    Selection,
}
pub trait InventoryRules<I, K, V> {
    fn is_enchanted(&self, stack: &StackRef<I, K, V>) -> bool;
    fn is_damaged(&self, stack: &StackRef<I, K, V>) -> bool;
    fn has_custom_name(&self, stack: &StackRef<I, K, V>) -> bool;
}
#[derive(Debug)]
pub struct PlayerInventory<I, K, V> {
    items: [StackRef<I, K, V>; 36],
    equipment: EntityEquipment<I, K, V>,
    stacks: ItemStacks<I, K, V>,
    selected: usize,
    pub times_changed: i32,
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq> PlayerInventory<I, K, V> {
    pub fn new(stacks: ItemStacks<I, K, V>, equipment: EntityEquipment<I, K, V>) -> Self {
        Self {
            items: std::array::from_fn(|_| stacks.empty()),
            equipment,
            stacks,
            selected: 0,
            times_changed: 0,
        }
    }
    pub(crate) fn stacks(&self) -> &ItemStacks<I, K, V> {
        &self.stacks
    }
    pub(crate) fn set_non_equipment(
        &mut self,
        index: i32,
        stack: StackRef<I, K, V>,
    ) -> Result<(), InventoryError> {
        let index = usize::try_from(index)
            .ok()
            .filter(|i| *i < 36)
            .ok_or(InventoryError::Index)?;
        self.items[index] = stack;
        Ok(())
    }
    pub fn items(&self) -> &[StackRef<I, K, V>; 36] {
        &self.items
    }
    pub fn equipment(&self) -> &EntityEquipment<I, K, V> {
        &self.equipment
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn set_selected(&mut self, selected: i32) -> Result<(), InventoryError> {
        if !(0..9).contains(&selected) {
            return Err(InventoryError::Selection);
        }
        self.selected = selected as usize;
        Ok(())
    }
    pub fn selected_item(&self) -> StackRef<I, K, V> {
        self.items[self.selected].clone()
    }
    pub fn set_selected_item(&mut self, stack: StackRef<I, K, V>) -> StackRef<I, K, V> {
        std::mem::replace(&mut self.items[self.selected], stack)
    }
    pub fn get(&self, index: i32) -> Result<StackRef<I, K, V>, InventoryError> {
        if index < 0 {
            return Err(InventoryError::Index);
        }
        Ok(if index < 36 {
            self.items[index as usize].clone()
        } else {
            EquipmentSlot::from_inventory_slot(index)
                .map_or_else(|| self.stacks.empty(), |s| self.equipment.get(s))
        })
    }
    pub fn set(&mut self, index: i32, stack: StackRef<I, K, V>) -> Result<(), InventoryError> {
        if index < 0 {
            return Err(InventoryError::Index);
        }
        if index < 36 {
            self.items[index as usize] = stack;
        } else if let Some(s) = EquipmentSlot::from_inventory_slot(index) {
            self.equipment.set(s, stack);
        }
        Ok(())
    }
    pub fn remove(&mut self, index: i32, amount: i32) -> StackRef<I, K, V> {
        if index < 0 || (index < 36 && amount <= 0) {
            return self.stacks.empty();
        }
        let s = self.get(index).expect("nonnegative index");
        if s.read().is_empty() {
            self.stacks.empty()
        } else {
            self.stacks.split(&s, amount)
        }
    }
    pub fn remove_no_update(&mut self, index: i32) -> Result<StackRef<I, K, V>, InventoryError> {
        if index < 0 {
            return Err(InventoryError::Index);
        }
        Ok(if index < 36 {
            std::mem::replace(&mut self.items[index as usize], self.stacks.empty())
        } else {
            EquipmentSlot::from_inventory_slot(index).map_or_else(
                || self.stacks.empty(),
                |s| self.equipment.set(s, self.stacks.empty()),
            )
        })
    }
    /// The native Map.of equipment iteration order varies by JVM. The caller
    /// supplies its pinned runtime order when reference aliases span equipment.
    pub fn remove_reference(
        &mut self,
        stack: &StackRef<I, K, V>,
        equipment_order: &[EquipmentSlot; 7],
    ) {
        for s in &mut self.items {
            if s.same_reference(stack) {
                *s = self.stacks.empty();
                return;
            }
        }
        for slot in equipment_order {
            if self.equipment.get(*slot).same_reference(stack) {
                self.equipment.set(*slot, self.stacks.empty());
                return;
            }
        }
    }
    pub fn free_slot(&self) -> i32 {
        self.items
            .iter()
            .position(|s| s.read().is_empty())
            .map_or(-1, |i| i as i32)
    }
    pub fn find_matching(&self, stack: &StackRef<I, K, V>) -> i32 {
        self.items
            .iter()
            .position(|s| !s.read().is_empty() && stack.read().same_item_components(&s.read()))
            .map_or(-1, |i| i as i32)
    }
    pub fn contains(&self, stack: &StackRef<I, K, V>) -> bool {
        (0..43).any(|i| {
            let s = self.get(i).expect("valid slot");
            !s.read().is_empty() && s.read().same_item_components(&stack.read())
        })
    }
    pub fn count_item(&self, item: I) -> i32 {
        (0..43).fold(0_i32, |n, i| {
            let s = self.get(i).expect("valid slot");
            let s = s.read();
            if s.item() == item {
                n.wrapping_add(s.count())
            } else {
                n
            }
        })
    }
    pub fn suitable_hotbar<R: InventoryRules<I, K, V>>(&self, rules: &R) -> usize {
        for delta in 0..9 {
            let i = (self.selected + delta) % 9;
            if self.items[i].read().is_empty() {
                return i;
            }
        }
        for delta in 0..9 {
            let i = (self.selected + delta) % 9;
            if !rules.is_enchanted(&self.items[i]) {
                return i;
            }
        }
        self.selected
    }
    pub fn find_crafting<R: InventoryRules<I, K, V>>(
        &self,
        item: I,
        stack: &StackRef<I, K, V>,
        rules: &R,
    ) -> i32 {
        self.items
            .iter()
            .position(|s| {
                !s.read().is_empty()
                    && s.read().item() == item
                    && !rules.is_damaged(s)
                    && !rules.is_enchanted(s)
                    && !rules.has_custom_name(s)
                    && (stack.read().is_empty() || stack.read().same_item_components(&s.read()))
            })
            .map_or(-1, |i| i as i32)
    }
    pub fn add_and_pick<R: InventoryRules<I, K, V>>(
        &mut self,
        stack: StackRef<I, K, V>,
        rules: &R,
    ) {
        self.selected = self.suitable_hotbar(rules);
        if !self.items[self.selected].read().is_empty() {
            let free = self.free_slot();
            if free >= 0 {
                self.items[free as usize] = self.items[self.selected].clone();
            }
        }
        self.items[self.selected] = stack;
    }
    pub fn pick_slot<R: InventoryRules<I, K, V>>(
        &mut self,
        index: i32,
        rules: &R,
    ) -> Result<(), InventoryError> {
        self.selected = self.suitable_hotbar(rules);
        // Selection changes even when the following list access fails.
        let index = usize::try_from(index)
            .ok()
            .filter(|i| *i < 36)
            .ok_or(InventoryError::Index)?;
        self.items.swap(self.selected, index);
        Ok(())
    }
    pub fn remove_selected(&mut self, all: bool) -> StackRef<I, K, V> {
        let s = self.selected_item();
        if s.read().is_empty() {
            return self.stacks.empty();
        }
        let amount = if all { s.read().count() } else { 1 };
        self.remove(self.selected as i32, amount)
    }
    pub fn changed(&mut self) {
        self.times_changed = self.times_changed.wrapping_add(1);
    }
    pub fn clear(&mut self) {
        self.items.fill(self.stacks.empty());
        self.equipment.clear();
    }
    pub fn is_empty(&self) -> bool {
        (0..43).all(|i| self.get(i).expect("valid slot").read().is_empty())
    }
    pub fn replace_self(&mut self) {
        for i in 0..43 {
            self.set(i, self.get(i).expect("valid slot"))
                .expect("valid slot");
        }
    }
    pub fn replace_with(&mut self, other: &Self) {
        for i in 0..43 {
            self.set(i, other.get(i).expect("valid slot"))
                .expect("valid slot");
        }
        self.selected = other.selected;
    }
}
