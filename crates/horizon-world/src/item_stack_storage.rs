//! Item-stack data kernels over component storage. Stable object identity belongs
//! to the containing item store; CopyResult::Empty means its canonical EMPTY.
use crate::component_patch::{ComponentMap, Patch, Prototype};
#[derive(Clone, Debug)]
pub struct ItemStackData<I, K, V> {
    item: I,
    air: I,
    pub count: i32,
    pub pop_time: i32,
    pub components: ComponentMap<K, V>,
}
#[derive(Clone, Debug)]
pub enum CopyResult<T> {
    Empty,
    Stack(T),
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq> ItemStackData<I, K, V> {
    pub fn new(item: I, air: I, count: i32, prototype: Prototype<K, V>) -> Self {
        Self {
            item,
            air,
            count,
            pop_time: 0,
            components: ComponentMap::new(prototype),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.item == self.air || self.count <= 0
    }
    pub fn stored_item(&self) -> I {
        self.item
    }
    pub fn item(&self) -> I {
        if self.is_empty() { self.air } else { self.item }
    }
    pub fn count(&self) -> i32 {
        if self.is_empty() { 0 } else { self.count }
    }
    pub fn component(&self, key: &K) -> Option<&V> {
        if self.is_empty() {
            None
        } else {
            self.components.get(key)
        }
    }
    pub fn visible_patch(&self) -> Patch<K, V> {
        if self.is_empty() {
            Patch::new([])
        } else {
            self.components.as_patch()
        }
    }
    pub fn copy(&self) -> CopyResult<Self> {
        if self.is_empty() {
            CopyResult::Empty
        } else {
            CopyResult::Stack(self.clone())
        }
    }
    pub fn copy_with_count(&self, count: i32) -> CopyResult<Self> {
        match self.copy() {
            CopyResult::Empty => CopyResult::Empty,
            CopyResult::Stack(mut s) => {
                s.count = count;
                CopyResult::Stack(s)
            }
        }
    }
    pub fn copy_and_clear(&mut self) -> CopyResult<Self> {
        if self.is_empty() {
            return CopyResult::Empty;
        }
        let copy = self.copy();
        self.count = 0;
        copy
    }
    pub fn grow(&mut self, amount: i32) {
        self.count = self.count().wrapping_add(amount);
    }
    pub fn shrink(&mut self, amount: i32) {
        self.grow(amount.wrapping_neg());
    }
    pub fn limit_size(&mut self, limit: i32) {
        if !self.is_empty() && self.count() > limit {
            self.count = limit;
        }
    }
    pub fn split(&mut self, amount: i32) -> CopyResult<Self> {
        let amount = amount.min(self.count());
        let copy = self.copy_with_count(amount);
        self.shrink(amount);
        copy
    }
    pub fn transmute_copy(
        &self,
        item: I,
        prototype: Prototype<K, V>,
        count: i32,
    ) -> CopyResult<Self> {
        if self.is_empty() {
            return CopyResult::Empty;
        }
        CopyResult::Stack(Self {
            item,
            air: self.air,
            count,
            pop_time: 0,
            components: ComponentMap::from_patch(prototype, &self.components.as_patch()),
        })
    }
    pub fn same_item(&self, other: &Self) -> bool {
        self.item() == other.item()
    }
    pub fn same_item_components(&self, other: &Self) -> bool {
        self.same_item(other)
            && ((self.is_empty() && other.is_empty()) || self.components == other.components)
    }
    pub fn matches(&self, other: &Self) -> bool {
        self.count() == other.count() && self.same_item_components(other)
    }
    pub fn consume<E>(
        &mut self,
        amount: i32,
        has_actor: bool,
        infinite: impl FnOnce(&mut Self) -> Result<bool, E>,
    ) -> Result<(), E> {
        if !has_actor || !infinite(self)? {
            self.shrink(amount);
        }
        Ok(())
    }
    pub fn consume_and_return<E>(
        &mut self,
        amount: i32,
        has_actor: bool,
        infinite: impl FnOnce(&mut Self) -> Result<bool, E>,
    ) -> Result<CopyResult<Self>, E> {
        let copy = self.copy_with_count(amount);
        self.consume(amount, has_actor, infinite)?;
        Ok(copy)
    }
}
