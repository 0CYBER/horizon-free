//! Source ownership and scalar preconditions for one equipment decision.
use crate::items::ItemStack;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);

/// A source location and version, usable only within its owning decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipStamp {
    location: u64,
    identity: u64,
    revision: u64,
    scope: u64,
}

impl EquipStamp {
    pub(super) fn fresh(location: u64) -> Self {
        let scope = NEXT_SCOPE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
            .unwrap_or(0);
        Self {
            location,
            identity: 1,
            revision: 0,
            scope,
        }
    }
    pub(super) fn valid(self) -> bool {
        self.scope != 0
    }
    pub fn scope(self) -> u64 {
        self.scope
    }
}

/// Residency and observation version of the target used to decide an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipEntityStamp {
    pub world: u64,
    pub id: u32,
    pub generation: u64,
    pub revision: u64,
}

/// A predicate answer retained until apply, with all of its preconditions.
#[derive(Clone, Copy, Debug)]
pub struct EquipAnswer<MenuStamp> {
    source: EquipStamp,
    menu: MenuStamp,
    entity: EquipEntityStamp,
    allowed: bool,
}

/// Owns mutable access to a source slot for the lifetime of one decision.
///
/// No mutable reference to the whole stack or slot escapes: mutations use the
/// field borrows below, and replacement/removal use `replace`/`remove`.
/// Merely acquiring a field borrow invalidates earlier preparations. A retired
/// scope still permits ordinary inventory edits, but never answers or applies.
pub struct EquipDecision<'a> {
    source: &'a mut Option<ItemStack>,
    stamp: EquipStamp,
    retired: bool,
    address: Option<usize>,
}
impl<'a> EquipDecision<'a> {
    pub fn new(location: u64, source: &'a mut Option<ItemStack>) -> Self {
        let stamp = EquipStamp::fresh(location);
        let address = source.as_ref().map(|stack| stack as *const _ as usize);
        if let Some(address) = address {
            super::source::publish(address, stamp.valid().then_some(stamp));
        }
        Self {
            source,
            stamp,
            retired: !stamp.valid(),
            address,
        }
    }
    pub fn stack(&self) -> Option<&ItemStack> {
        self.source.as_ref()
    }
    pub fn stamp(&self) -> Option<EquipStamp> {
        (!self.retired && self.source.is_some()).then_some(self.stamp)
    }
    pub fn matches(&self, stamp: EquipStamp) -> bool {
        self.stamp() == Some(stamp)
    }
    fn invalidate(&mut self, identity: bool) {
        match self.stamp.revision.checked_add(1) {
            Some(next) => self.stamp.revision = next,
            None => self.retired = true,
        }
        if identity {
            match self.stamp.identity.checked_add(1) {
                Some(next) => self.stamp.identity = next,
                None => self.retired = true,
            }
        }
        if let Some(address) = self.address {
            super::source::publish(address, self.stamp());
        }
    }
    pub fn count_mut(&mut self) -> Option<&mut u32> {
        self.invalidate(false);
        self.source.as_mut().map(|stack| &mut stack.count)
    }
    pub fn damage_mut(&mut self) -> Option<&mut u32> {
        self.invalidate(false);
        self.source.as_mut().map(|stack| &mut stack.damage)
    }
    pub fn item_mut(&mut self) -> Option<&mut std::sync::Arc<str>> {
        self.invalidate(false);
        self.source.as_mut().map(|stack| &mut stack.item)
    }
    pub fn data_mut(&mut self) -> Option<&mut horizon_items::item_data::StackData> {
        self.invalidate(false);
        self.source.as_mut().map(ItemStack::data_mut)
    }
    pub fn replace(&mut self, stack: Option<ItemStack>) -> Option<ItemStack> {
        self.invalidate(true);
        let old = std::mem::replace(self.source, stack);
        if self.address.is_none() {
            self.address = self.source.as_ref().map(|stack| stack as *const _ as usize);
        }
        if let Some(address) = self.address {
            super::source::publish(address, self.stamp());
        }
        old
    }
    pub fn split(&mut self, count: u32) -> ItemStack {
        let mut result = self.stack().expect("equipment source removed").clone();
        let count = count.min(result.count);
        result.count = count;
        *self.count_mut().expect("equipment source removed") -= count;
        result
    }
    pub fn remove(&mut self) -> Option<ItemStack> {
        self.replace(None)
    }
    pub fn ask(&self, stamp: EquipStamp, callback: impl FnOnce(&ItemStack) -> bool) -> bool {
        if !self.matches(stamp) {
            return false;
        }
        self.stack().is_some_and(callback)
    }
    pub fn answer<MenuStamp>(
        &self,
        stamp: EquipStamp,
        menu: MenuStamp,
        entity: EquipEntityStamp,
        allowed: bool,
    ) -> Option<EquipAnswer<MenuStamp>> {
        self.matches(stamp).then_some(EquipAnswer {
            source: stamp,
            menu,
            entity,
            allowed,
        })
    }
    pub fn apply<MenuStamp: PartialEq, R>(
        &mut self,
        answer: EquipAnswer<MenuStamp>,
        menu: MenuStamp,
        entity: Option<EquipEntityStamp>,
        apply: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if !answer.allowed
            || !self.matches(answer.source)
            || answer.menu != menu
            || entity != Some(answer.entity)
        {
            return None;
        }
        Some(apply(self))
    }
}

#[cfg(test)]
#[path = "equip_decision_tests.rs"]
mod tests;

impl std::ops::Deref for EquipDecision<'_> {
    type Target = ItemStack;
    fn deref(&self) -> &ItemStack {
        self.stack().expect("equipment source removed")
    }
}
impl Drop for EquipDecision<'_> {
    fn drop(&mut self) {
        if let Some(address) = self.address {
            super::source::forget(address);
        }
    }
}
