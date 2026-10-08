//! Item cooldown storage with native hash-map expiry order and callback prefixes.
use crate::java_hash_map::{CursorError, JavaHashMap, JavaKey};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CooldownInstance {
    pub start: i32,
    pub end: i32,
}
pub struct ItemCooldowns<K> {
    pub tick_count: i32,
    entries: JavaHashMap<K, CooldownInstance>,
}
pub trait Observer<K: JavaKey + Clone> {
    type Error;
    fn started(
        &mut self,
        cooldowns: &mut ItemCooldowns<K>,
        group: K,
        duration: i32,
    ) -> Result<(), Self::Error>;
    fn ended(&mut self, cooldowns: &mut ItemCooldowns<K>, group: K) -> Result<(), Self::Error>;
}
#[derive(Debug)]
pub enum TickError<E> {
    Iterator(CursorError),
    Observer(E),
}
impl<K: JavaKey + Clone> Default for ItemCooldowns<K> {
    fn default() -> Self {
        Self {
            tick_count: 0,
            entries: JavaHashMap::default(),
        }
    }
}
impl<K: JavaKey + Clone> ItemCooldowns<K> {
    pub fn entries(&self) -> impl Iterator<Item = (&K, &CooldownInstance)> {
        self.entries.entries()
    }
    pub fn percent(&self, group: &K, partial_tick: f32) -> f32 {
        let Some(instance) = self.entries.get(group) else {
            return 0.0;
        };
        let duration = instance.end.wrapping_sub(instance.start) as f32;
        let remaining = instance.end as f32 - (self.tick_count as f32 + partial_tick);
        (remaining / duration).clamp(0.0, 1.0)
    }
    pub fn is_on_cooldown(&self, group: &K) -> bool {
        self.percent(group, 0.0) > 0.0
    }
    pub fn add<O: Observer<K>>(
        &mut self,
        group: K,
        duration: i32,
        observer: &mut O,
    ) -> Result<(), O::Error> {
        self.entries.put(
            group.clone(),
            CooldownInstance {
                start: self.tick_count,
                end: self.tick_count.wrapping_add(duration),
            },
        );
        observer.started(self, group, duration)
    }
    pub fn remove<O: Observer<K>>(&mut self, group: K, observer: &mut O) -> Result<(), O::Error> {
        self.entries.remove(&group);
        observer.ended(self, group)
    }
    pub fn tick<O: Observer<K>>(&mut self, observer: &mut O) -> Result<(), TickError<O::Error>> {
        self.tick_count = self.tick_count.wrapping_add(1);
        let mut cursor = self.entries.cursor();
        while cursor.has_next() {
            let (key, instance) = self
                .entries
                .next_entry(&mut cursor)
                .map_err(TickError::Iterator)?
                .unwrap();
            if instance.end <= self.tick_count {
                let key = key.clone();
                self.entries
                    .remove_current(&mut cursor)
                    .map_err(TickError::Iterator)?;
                observer.ended(self, key).map_err(TickError::Observer)?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct UseCooldown<K> {
    pub seconds: f32,
    pub group: Option<K>,
}
impl<K: PartialEq> PartialEq for UseCooldown<K> {
    fn eq(&self, other: &Self) -> bool {
        let bits = |v: f32| if v.is_nan() { 0x7fc00000 } else { v.to_bits() };
        bits(self.seconds) == bits(other.seconds) && self.group == other.group
    }
}
impl<K> UseCooldown<K> {
    pub fn ticks(&self) -> i32 {
        (self.seconds * 20.0) as i32
    }
}
/// The registered item identifier is fetched even when the component has a group.
pub fn group<K: Clone>(
    component: Option<&UseCooldown<K>>,
    item_identifier: impl FnOnce() -> K,
) -> K {
    let item = item_identifier();
    component.and_then(|c| c.group.clone()).unwrap_or(item)
}

/// The player query is a class test; retrieving its cooldown manager is virtual.
pub trait CooldownUser {
    type Error;
    type Stack;
    type Manager;
    fn is_player(&self) -> bool;
    fn cooldowns(&mut self) -> Result<Self::Manager, Self::Error>;
    fn add_cooldown(
        &mut self,
        manager: Self::Manager,
        stack: Self::Stack,
        ticks: i32,
    ) -> Result<(), Self::Error>;
}
impl<K> UseCooldown<K> {
    pub fn apply<W: CooldownUser>(&self, user: &mut W, stack: W::Stack) -> Result<(), W::Error> {
        if user.is_player() {
            let manager = user.cooldowns()?;
            user.add_cooldown(manager, stack, self.ticks())?;
        }
        Ok(())
    }
}
