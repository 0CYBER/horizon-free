//! Shared stack identity for inventories. Cloning a reference retains the same
//! stack; `ItemStacks::copy` copies its value with copy-on-write components.
//! These references belong to the simulation thread and release unreachable
//! stacks automatically. No borrow is held across a container callback.
use crate::{
    component_patch::Prototype,
    item_stack_storage::{CopyResult, ItemStackData},
};
use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};
#[derive(Debug)]
pub struct StackRef<I, K, V>(Rc<RefCell<ItemStackData<I, K, V>>>);
impl<I, K, V> Clone for StackRef<I, K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
// Native ItemStack object equality is reference identity. Value comparisons
// remain explicit on ItemStackData (same_item_components/matches).
impl<I, K, V> PartialEq for StackRef<I, K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.same_reference(other)
    }
}
impl<I, K, V> Eq for StackRef<I, K, V> {}
impl<I, K, V> StackRef<I, K, V> {
    pub fn new(data: ItemStackData<I, K, V>) -> Self {
        Self(Rc::new(RefCell::new(data)))
    }
    pub fn read(&self) -> Ref<'_, ItemStackData<I, K, V>> {
        self.0.borrow()
    }
    pub fn write(&self) -> RefMut<'_, ItemStackData<I, K, V>> {
        self.0.borrow_mut()
    }
    pub fn same_reference(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
#[derive(Clone, Debug)]
pub struct ItemStacks<I, K, V> {
    air: I,
    empty: StackRef<I, K, V>,
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq> ItemStacks<I, K, V> {
    pub fn new(air: I) -> Self {
        Self {
            air,
            empty: StackRef::new(ItemStackData::new(air, air, 0, Prototype::new([]))),
        }
    }
    pub fn empty(&self) -> StackRef<I, K, V> {
        self.empty.clone()
    }
    pub fn create(&self, item: I, count: i32, prototype: Prototype<K, V>) -> StackRef<I, K, V> {
        StackRef::new(ItemStackData::new(item, self.air, count, prototype))
    }
    pub fn resolve(&self, copy: CopyResult<ItemStackData<I, K, V>>) -> StackRef<I, K, V> {
        match copy {
            CopyResult::Empty => self.empty(),
            CopyResult::Stack(data) => StackRef::new(data),
        }
    }
    pub fn copy(&self, stack: &StackRef<I, K, V>) -> StackRef<I, K, V> {
        self.resolve(stack.read().copy())
    }
    pub fn split(&self, stack: &StackRef<I, K, V>, amount: i32) -> StackRef<I, K, V> {
        self.resolve(stack.write().split(amount))
    }
}
