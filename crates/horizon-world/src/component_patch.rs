//! Shared immutable prototypes with ordered, copy-on-write component patches.
//! Keys stand for stable component-type identities. Callers supply prototype
//! iteration order and values whose equality matches their native component.
use std::{fmt, sync::Arc};
#[derive(Clone, Debug)]
pub struct Prototype<K, V>(Arc<Vec<(K, V)>>);
#[derive(Clone, Debug)]
pub struct Patch<K, V>(Arc<Vec<(K, Option<V>)>>);
#[derive(Clone, Debug)]
pub struct ComponentMap<K, V> {
    prototype: Prototype<K, V>,
    patch: Patch<K, V>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingPrototype;
impl fmt::Display for MissingPrototype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("removed component has no prototype value")
    }
}
impl std::error::Error for MissingPrototype {}
fn index<K: Eq, V>(entries: &[(K, V)], key: &K) -> Option<usize> {
    entries.iter().position(|(k, _)| k == key)
}
fn put<K: Eq, V>(entries: &mut Vec<(K, V)>, key: K, value: V) -> Option<V> {
    if let Some(i) = index(entries, &key) {
        Some(std::mem::replace(&mut entries[i].1, value))
    } else {
        entries.push((key, value));
        None
    }
}
fn remove<K: Eq, V>(entries: &mut Vec<(K, V)>, key: &K) -> Option<V> {
    index(entries, key).map(|i| entries.remove(i).1)
}
fn map_equal<K: Eq, V: PartialEq>(a: &[(K, V)], b: &[(K, V)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| index(b, k).is_some_and(|i| b[i].1 == *v))
}
impl<K: Clone + Eq, V: Clone + PartialEq> Prototype<K, V> {
    pub fn new(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        let mut values = Vec::new();
        for (k, v) in entries {
            put(&mut values, k, v);
        }
        Self(Arc::new(values))
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        index(&self.0, key).map(|i| &self.0[i].1)
    }
    pub fn entries(&self) -> &[(K, V)] {
        &self.0
    }
}
impl<K: Clone + Eq, V: Clone + PartialEq> Patch<K, V> {
    /// None is an explicit removal, distinct from a missing patch entry.
    pub fn new(entries: impl IntoIterator<Item = (K, Option<V>)>) -> Self {
        let mut values = Vec::new();
        for (k, v) in entries {
            put(&mut values, k, v);
        }
        Self(Arc::new(values))
    }
    pub fn entries(&self) -> &[(K, Option<V>)] {
        &self.0
    }
}
impl<K: Clone + Eq, V: Clone + PartialEq> ComponentMap<K, V> {
    pub fn new(prototype: Prototype<K, V>) -> Self {
        Self {
            prototype,
            patch: Patch(Arc::new(Vec::new())),
        }
    }
    pub fn from_patch(prototype: Prototype<K, V>, patch: &Patch<K, V>) -> Self {
        let sanitized = patch.0.iter().all(|(k, v)| match v {
            Some(v) => Some(v) != prototype.get(k),
            None => prototype.get(k).is_some(),
        });
        if sanitized {
            Self {
                prototype,
                patch: patch.clone(),
            }
        } else {
            let mut result = Self::new(prototype);
            result.apply_patch(patch);
            result
        }
    }
    pub fn prototype(&self) -> &Prototype<K, V> {
        &self.prototype
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        match index(&self.patch.0, key) {
            Some(i) => self.patch.0[i].1.as_ref(),
            None => self.prototype.get(key),
        }
    }
    pub fn has_non_default(&self, key: &K) -> bool {
        index(&self.patch.0, key).is_some()
    }
    /// Vanilla set returns the prototype value for an earlier removal marker.
    /// An unsanitized removal with no prototype causes an error after mutation.
    pub fn set(&mut self, key: K, value: Option<V>) -> Result<Option<V>, MissingPrototype> {
        let base = self.prototype.get(&key).cloned();
        let entries = Arc::make_mut(&mut self.patch.0);
        let previous = if value == base {
            remove(entries, &key)
        } else {
            put(entries, key, value)
        };
        match previous {
            Some(Some(v)) => Ok(Some(v)),
            Some(None) => base.map(Some).ok_or(MissingPrototype),
            None => Ok(base),
        }
    }
    pub fn remove(&mut self, key: K) -> Option<V> {
        let base = self.prototype.get(&key).cloned();
        let entries = Arc::make_mut(&mut self.patch.0);
        let previous = if base.is_some() {
            put(entries, key, None)
        } else {
            remove(entries, &key)
        };
        previous.unwrap_or(base)
    }
    pub fn apply_patch(&mut self, patch: &Patch<K, V>) {
        let entries = Arc::make_mut(&mut self.patch.0);
        for (k, v) in patch.0.iter() {
            if v.as_ref() == self.prototype.get(k) {
                remove(entries, k);
            } else {
                put(entries, k.clone(), v.clone());
            }
        }
    }
    /// Restore preserves redundant values and removals, unlike apply_patch.
    pub fn restore_patch(&mut self, patch: &Patch<K, V>) {
        self.patch = patch.clone();
    }
    pub fn clear_patch(&mut self) {
        self.patch = Patch(Arc::new(Vec::new()));
    }
    pub fn as_patch(&self) -> Patch<K, V> {
        self.patch.clone()
    }
    pub fn len(&self) -> usize {
        self.entries().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Patched values precede untouched prototype values; removed keys vanish.
    pub fn entries(&self) -> impl Iterator<Item = (&K, &V)> {
        self.patch
            .0
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
            .chain(
                self.prototype
                    .0
                    .iter()
                    .filter(|(k, _)| !self.has_non_default(k))
                    .map(|(k, v)| (k, v)),
            )
    }
    /// Key-set order instead starts with prototype keys, then applies the patch.
    pub fn keys(&self) -> Vec<K> {
        let mut keys: Vec<_> = self.prototype.0.iter().map(|(k, _)| k.clone()).collect();
        for (k, v) in self.patch.0.iter() {
            let i = keys.iter().position(|key| key == k);
            if v.is_some() {
                if i.is_none() {
                    keys.push(k.clone());
                }
            } else if let Some(i) = i {
                keys.remove(i);
            }
        }
        keys
    }
}
impl<K: Eq, V: PartialEq> PartialEq for ComponentMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        map_equal(&self.prototype.0, &other.prototype.0) && map_equal(&self.patch.0, &other.patch.0)
    }
}

impl<K: Eq, V: PartialEq> PartialEq for Patch<K, V> {
    fn eq(&self, other: &Self) -> bool {
        map_equal(&self.0, &other.0)
    }
}
