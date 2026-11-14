//! Active source versions for equipment questions. Addresses stay host-local.
use super::decision::EquipStamp;
use crate::items::ItemStack;
use std::{cell::RefCell, collections::HashMap, ops::Deref};
thread_local! { static SOURCES: RefCell<HashMap<usize,Option<EquipStamp>>> = RefCell::default(); }
pub(super) fn publish(address: usize, stamp: Option<EquipStamp>) {
    SOURCES.with(|sources| {
        sources.borrow_mut().insert(address, stamp);
    });
}
pub(super) fn forget(address: usize) {
    SOURCES.with(|sources| {
        sources.borrow_mut().remove(&address);
    });
}
pub fn source_stamp(stack: &ItemStack) -> Option<EquipStamp> {
    source_stamp_at(stack as *const _ as usize).flatten()
}
pub fn source_stamp_at(address: usize) -> Option<Option<EquipStamp>> {
    SOURCES.with(|sources| sources.borrow().get(&address).copied())
}
/// A frozen menu input. It cannot hand out mutable stack references.
pub struct EquipInput {
    stack: Box<ItemStack>,
    stamp: EquipStamp,
}
impl EquipInput {
    pub fn new(location: u64, stack: ItemStack) -> Self {
        let stack = Box::new(stack);
        let stamp = EquipStamp::fresh(location);
        publish(&*stack as *const _ as usize, stamp.valid().then_some(stamp));
        Self { stack, stamp }
    }
    pub fn stamp(&self) -> Option<EquipStamp> {
        self.stamp.valid().then_some(self.stamp)
    }
}
impl Deref for EquipInput {
    type Target = ItemStack;
    fn deref(&self) -> &ItemStack {
        &self.stack
    }
}
impl Drop for EquipInput {
    fn drop(&mut self) {
        forget(&*self.stack as *const _ as usize);
    }
}
