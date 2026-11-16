//! Stable storage identity for a menu's owning slot, including while lifted.
//!
//! A menu value's stamp cannot detect taking that same value out and putting it
//! back. This owner advances on every mutable access, even an equal replacement
//! or an unwind. Action admission uses its stable id before any take occurs.
use super::{Generation, MenuId};
use std::ops::{Deref, DerefMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) struct OwnerStamp {
    pub(super) id: MenuId,
    revision: Generation,
}

pub(in crate::game) struct MenuOwner<T> {
    stamp: OwnerStamp,
    value: T,
}

impl<T: Default> Default for MenuOwner<T> {
    fn default() -> Self {
        Self {
            stamp: OwnerStamp {
                id: MenuId::fresh(),
                revision: Generation::default(),
            },
            value: T::default(),
        }
    }
}

impl<T> MenuOwner<T> {
    pub(in crate::game) fn id(&self) -> MenuId {
        self.stamp.id
    }

    pub(in crate::game) fn stamp(&self) -> OwnerStamp {
        self.stamp
    }

    /// Undo content only. The owner and its revision must never roll back.
    pub(in crate::game) fn restore(&mut self, value: T) {
        self.stamp.revision.advance();
        self.value = value;
    }
}

impl<T> Deref for MenuOwner<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for MenuOwner<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.stamp.revision.advance();
        &mut self.value
    }
}

#[cfg(test)]
#[path = "menu_owner_tests.rs"]
mod tests;
