//! Durability backed by native-style component prototypes and patches.
//! Other item components and item identity are supplied by the containing stack.
use crate::{
    component_patch::{ComponentMap, MissingPrototype, Patch, Prototype},
    item_durability::DurabilityStorage,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    MaxDamage,
    Damage,
    Unbreakable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    MaxDamage(Option<i32>),
    Damage(Option<i32>),
    Unbreakable(bool),
}
impl Change {
    fn parts(self) -> (Key, Option<i32>) {
        match self {
            Self::MaxDamage(v) => (Key::MaxDamage, v),
            Self::Damage(v) => (Key::Damage, v),
            Self::Unbreakable(v) => (Key::Unbreakable, v.then_some(0)),
        }
    }
    fn from_parts(key: Key, v: Option<i32>) -> Self {
        match key {
            Key::MaxDamage => Self::MaxDamage(v),
            Key::Damage => Self::Damage(v),
            Key::Unbreakable => Self::Unbreakable(v.is_some()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct DurabilityPrototype(Prototype<Key, i32>);
impl DurabilityPrototype {
    pub fn new(changes: impl IntoIterator<Item = Change>) -> Self {
        let mut entries = Vec::new();
        for change in changes {
            let (k, v) = change.parts();
            let i = entries.iter().position(|(key, _)| *key == k);
            match (v, i) {
                (Some(v), Some(i)) => entries[i] = (k, v),
                (Some(v), None) => entries.push((k, v)),
                (None, Some(i)) => {
                    entries.remove(i);
                }
                (None, None) => {}
            }
        }
        Self(Prototype::new(entries))
    }
}
#[derive(Clone, Debug)]
pub struct DurabilityPatch(Patch<Key, i32>);
impl DurabilityPatch {
    pub fn new(changes: impl IntoIterator<Item = Change>) -> Self {
        Self(Patch::new(changes.into_iter().map(Change::parts)))
    }
    pub fn changes(&self) -> impl Iterator<Item = Change> + '_ {
        self.0
            .entries()
            .iter()
            .map(|(k, v)| Change::from_parts(*k, *v))
    }
}
#[derive(Clone, Debug)]
pub struct ComponentDurability {
    pub count: i32,
    pub air: bool,
    components: ComponentMap<Key, i32>,
}
impl ComponentDurability {
    pub fn new(prototype: &DurabilityPrototype, count: i32, air: bool) -> Self {
        Self {
            count,
            air,
            components: ComponentMap::new(prototype.0.clone()),
        }
    }
    pub fn from_patch(
        prototype: &DurabilityPrototype,
        patch: &DurabilityPatch,
        count: i32,
        air: bool,
    ) -> Self {
        Self {
            count,
            air,
            components: ComponentMap::from_patch(prototype.0.clone(), &patch.0),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.air || self.count <= 0
    }
    pub fn count(&self) -> i32 {
        if self.is_empty() { 0 } else { self.count }
    }
    /// Raw storage remains present while the containing item stack is empty.
    pub fn raw_component(&self, key: Key) -> Option<i32> {
        self.components.get(&key).copied()
    }
    pub fn set(&mut self, change: Change) -> Result<Option<Change>, MissingPrototype> {
        let (k, v) = change.parts();
        self.components
            .set(k, v)
            .map(|old| old.map(|v| Change::from_parts(k, Some(v))))
    }
    pub fn remove(&mut self, key: Key) -> Option<Change> {
        self.components
            .remove(key)
            .map(|v| Change::from_parts(key, Some(v)))
    }
    pub fn apply_patch(&mut self, patch: &DurabilityPatch) {
        self.components.apply_patch(&patch.0);
    }
    pub fn restore_patch(&mut self, patch: &DurabilityPatch) {
        self.components.restore_patch(&patch.0);
    }
    pub fn as_patch(&self) -> DurabilityPatch {
        DurabilityPatch(self.components.as_patch())
    }
    pub fn max_damage(&self) -> i32 {
        if self.is_empty() {
            0
        } else {
            self.raw_component(Key::MaxDamage).unwrap_or(0)
        }
    }
    pub fn damage(&self) -> i32 {
        let value = if self.is_empty() {
            0
        } else {
            self.raw_component(Key::Damage).unwrap_or(0)
        };
        value.max(0).min(self.max_damage())
    }
    pub fn set_damage(&mut self, value: i32) -> Result<(), MissingPrototype> {
        self.set(Change::Damage(Some(value.max(0).min(self.max_damage()))))
            .map(|_| ())
    }
    pub fn is_damageable(&self) -> bool {
        !self.is_empty()
            && self.raw_component(Key::MaxDamage).is_some()
            && self.raw_component(Key::Unbreakable).is_none()
            && self.raw_component(Key::Damage).is_some()
    }
    pub fn is_damaged(&self) -> bool {
        self.is_damageable() && self.damage() > 0
    }
    pub fn is_broken(&self) -> bool {
        self.is_damageable() && self.damage() >= self.max_damage()
    }
    pub fn next_damage_will_break(&self) -> bool {
        self.is_damageable() && self.damage() >= self.max_damage().wrapping_sub(1)
    }
    pub fn grow(&mut self, amount: i32) {
        self.count = self.count().wrapping_add(amount);
    }
    pub fn shrink(&mut self, amount: i32) {
        self.grow(amount.wrapping_neg());
    }
}
impl DurabilityStorage for ComponentDurability {
    type Error = MissingPrototype;
    fn is_damageable(&self) -> bool {
        self.is_damageable()
    }
    fn damage(&self) -> i32 {
        self.damage()
    }
    fn max_damage(&self) -> i32 {
        self.max_damage()
    }
    fn set_damage(&mut self, value: i32) -> Result<(), Self::Error> {
        self.set_damage(value)
    }
    fn is_broken(&self) -> bool {
        self.is_broken()
    }
    fn shrink(&mut self, amount: i32) {
        self.shrink(amount);
    }
}
