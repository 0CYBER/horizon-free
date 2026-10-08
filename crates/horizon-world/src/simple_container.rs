//! SimpleContainer storage and transfers using shared stack references.
//! Item component semantics and the virtual size/change hooks are supplied by
//! the caller. Menu transactions, persistence and recipe accounting are separate.
use crate::{
    component_patch::Prototype,
    item_stack_refs::{ItemStacks, StackRef},
};
#[derive(Debug)]
pub enum ContainerError<E> {
    Index,
    Callback(E),
}
pub type ContainerResult<T, E> = Result<T, ContainerError<E>>;
#[derive(Debug)]
pub struct SimpleContainer<I, K, V> {
    items: Vec<StackRef<I, K, V>>,
    stacks: ItemStacks<I, K, V>,
}
pub trait ContainerHooks<I, K, V> {
    type Error;
    /// Native getMaxStackSize(), normally 99; called before the item's limit.
    fn max_stack_size(
        &mut self,
        container: &mut SimpleContainer<I, K, V>,
    ) -> Result<i32, Self::Error>;
    fn item_max_stack_size(&self, stack: &StackRef<I, K, V>) -> i32;
    fn changed(&mut self, container: &mut SimpleContainer<I, K, V>) -> Result<(), Self::Error>;
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq> SimpleContainer<I, K, V> {
    pub fn new(size: usize, stacks: ItemStacks<I, K, V>) -> Self {
        Self {
            items: vec![stacks.empty(); size],
            stacks,
        }
    }
    /// Like the native varargs constructor, this retains aliases without limits.
    pub fn from_items(items: Vec<StackRef<I, K, V>>, stacks: ItemStacks<I, K, V>) -> Self {
        Self { items, stacks }
    }
    pub fn items(&self) -> &[StackRef<I, K, V>] {
        &self.items
    }
    pub fn items_mut(&mut self) -> &mut [StackRef<I, K, V>] {
        &mut self.items
    }
    pub fn size(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|s| s.read().is_empty())
    }
    pub fn get(&self, index: i32) -> StackRef<I, K, V> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.items.get(i))
            .cloned()
            .unwrap_or_else(|| self.stacks.empty())
    }
    pub fn count_item(&self, item: I) -> i32 {
        self.items.iter().fold(0_i32, |total, s| {
            let s = s.read();
            if s.item() == item {
                total.wrapping_add(s.count())
            } else {
                total
            }
        })
    }
    pub fn can_add<H: ContainerHooks<I, K, V>>(
        &self,
        stack: &StackRef<I, K, V>,
        hooks: &H,
    ) -> bool {
        self.items.iter().any(|s| {
            s.read().is_empty()
                || (s.read().same_item_components(&stack.read())
                    && s.read().count() < hooks.item_max_stack_size(s))
        })
    }
    fn limit<H: ContainerHooks<I, K, V>>(
        &mut self,
        stack: &StackRef<I, K, V>,
        hooks: &mut H,
    ) -> Result<i32, ContainerError<H::Error>> {
        let container = hooks
            .max_stack_size(self)
            .map_err(ContainerError::Callback)?;
        Ok(container.min(hooks.item_max_stack_size(stack)))
    }
    pub fn set<H: ContainerHooks<I, K, V>>(
        &mut self,
        index: i32,
        stack: StackRef<I, K, V>,
        hooks: &mut H,
    ) -> Result<(), ContainerError<H::Error>> {
        let slot = usize::try_from(index)
            .ok()
            .and_then(|i| self.items.get_mut(i))
            .ok_or(ContainerError::Index)?;
        *slot = stack.clone();
        let limit = self.limit(&stack, hooks)?;
        stack.write().limit_size(limit);
        hooks.changed(self).map_err(ContainerError::Callback)
    }
    pub fn remove<H: ContainerHooks<I, K, V>>(
        &mut self,
        index: i32,
        amount: i32,
        hooks: &mut H,
    ) -> Result<StackRef<I, K, V>, ContainerError<H::Error>> {
        let s = self.get(index);
        let result = if amount > 0 && !s.read().is_empty() {
            self.stacks.split(&s, amount)
        } else {
            self.stacks.empty()
        };
        if !result.read().is_empty() {
            hooks.changed(self).map_err(ContainerError::Callback)?;
        }
        Ok(result)
    }
    pub fn remove_no_update<E>(
        &mut self,
        index: i32,
    ) -> Result<StackRef<I, K, V>, ContainerError<E>> {
        let slot = usize::try_from(index)
            .ok()
            .and_then(|i| self.items.get_mut(i))
            .ok_or(ContainerError::Index)?;
        if slot.read().is_empty() {
            Ok(self.stacks.empty())
        } else {
            Ok(std::mem::replace(slot, self.stacks.empty()))
        }
    }
    pub fn remove_type<H: ContainerHooks<I, K, V>>(
        &mut self,
        item: I,
        prototype: Prototype<K, V>,
        amount: i32,
        hooks: &mut H,
    ) -> Result<StackRef<I, K, V>, ContainerError<H::Error>> {
        let result = self.stacks.create(item, 0, prototype);
        for index in (0..self.size()).rev() {
            let s = self.items[index].clone();
            if s.read().item() == item {
                let wanted = amount.wrapping_sub(result.read().count());
                let removed = self.stacks.split(&s, wanted);
                result.write().grow(removed.read().count());
                if result.read().count() == amount {
                    break;
                }
            }
        }
        if !result.read().is_empty() {
            hooks.changed(self).map_err(ContainerError::Callback)?;
        }
        Ok(result)
    }
    pub fn clear<H: ContainerHooks<I, K, V>>(
        &mut self,
        hooks: &mut H,
    ) -> Result<(), ContainerError<H::Error>> {
        self.items.fill(self.stacks.empty());
        hooks.changed(self).map_err(ContainerError::Callback)
    }
    pub fn remove_all<H: ContainerHooks<I, K, V>>(
        &mut self,
        hooks: &mut H,
    ) -> ContainerResult<Vec<StackRef<I, K, V>>, H::Error> {
        let result = self
            .items
            .iter()
            .filter(|s| !s.read().is_empty())
            .cloned()
            .collect();
        self.clear(hooks)?;
        Ok(result)
    }
    pub fn add<H: ContainerHooks<I, K, V>>(
        &mut self,
        stack: &StackRef<I, K, V>,
        hooks: &mut H,
    ) -> Result<StackRef<I, K, V>, ContainerError<H::Error>> {
        if stack.read().is_empty() {
            return Ok(self.stacks.empty());
        }
        let remainder = self.stacks.copy(stack);
        for index in 0..self.size() {
            let target = self.items[index].clone();
            if target.read().same_item_components(&remainder.read()) {
                let limit = self.limit(&target, hooks)?;
                let amount = remainder
                    .read()
                    .count()
                    .min(limit.wrapping_sub(target.read().count()));
                if amount > 0 {
                    target.write().grow(amount);
                    remainder.write().shrink(amount);
                    hooks.changed(self).map_err(ContainerError::Callback)?;
                }
                if remainder.read().is_empty() {
                    return Ok(self.stacks.empty());
                }
            }
        }
        for index in 0..self.size() {
            if self.items[index].read().is_empty() {
                let moved = self.stacks.resolve(remainder.write().copy_and_clear());
                self.set(index as i32, moved, hooks)?;
                break;
            }
        }
        if remainder.read().is_empty() {
            Ok(self.stacks.empty())
        } else {
            Ok(remainder)
        }
    }
}
