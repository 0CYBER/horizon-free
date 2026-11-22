//! The samples mesh jobs read (`Snapshot::samples`): the active tiles' ready columns, kept
//! as the tiles change rather than gathered again for every batch of jobs. Kept in shards of
//! nearby tiles behind `Arc`s: a batch's copy (`clone`) copies the shards' handles, and a
//! change copies only its shard, and only while a job still reads it.
use super::{TileColumns, TileKey};
use crate::fast_hash::FxHashMap;
use std::sync::Arc;

type Shard = FxHashMap<TileKey, Arc<TileColumns>>;

/// 8 by 8 tiles of one level a shard.
fn shard_of(key: TileKey) -> (i32, i32, u8) {
    (key.x >> 3, key.z >> 3, key.level)
}

#[derive(Clone, Default)]
pub(super) struct Samples {
    shards: FxHashMap<(i32, i32, u8), Arc<Shard>>,
}

impl Samples {
    pub fn get(&self, key: &TileKey) -> Option<&Arc<TileColumns>> {
        self.shards.get(&shard_of(*key))?.get(key)
    }
    /// Sets `key`'s samples (`None`: not active or not ready), copying its shard only when
    /// they change.
    pub fn set(&mut self, key: TileKey, samples: Option<&Arc<TileColumns>>) {
        let at = shard_of(key);
        match samples {
            Some(samples) => {
                let shard = self.shards.entry(at).or_default();
                if shard.get(&key).is_none_or(|s| !Arc::ptr_eq(s, samples)) {
                    Arc::make_mut(shard).insert(key, samples.clone());
                }
            }
            None => {
                let Some(shard) = self.shards.get_mut(&at) else {
                    return;
                };
                if shard.contains_key(&key) {
                    Arc::make_mut(shard).remove(&key);
                    if shard.is_empty() {
                        self.shards.remove(&at);
                    }
                }
            }
        }
    }
    /// Drops `key`'s handle from its shard only when no snapshot shares that
    /// shard, letting `Lod::apply_detail` mutate a uniquely owned tile in
    /// place before `sync_sample` puts the handle back.
    pub fn release_if_unshared(&mut self, key: TileKey) {
        if let Some(shard) = self.shards.get_mut(&shard_of(key))
            && let Some(shard) = Arc::get_mut(shard)
        {
            shard.remove(&key);
        }
    }
    /// Keeps the tiles `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&TileKey) -> bool) {
        self.shards.retain(|_, shard| {
            if !shard.keys().all(&mut keep) {
                Arc::make_mut(shard).retain(|key, _| keep(key));
            }
            !shard.is_empty()
        });
    }
    #[cfg(any(test, debug_assertions))]
    pub fn iter(&self) -> impl Iterator<Item = (&TileKey, &Arc<TileColumns>)> {
        self.shards.values().flat_map(|shard| shard.iter())
    }
}

impl FromIterator<(TileKey, Arc<TileColumns>)> for Samples {
    fn from_iter<I: IntoIterator<Item = (TileKey, Arc<TileColumns>)>>(iter: I) -> Self {
        let mut samples = Self::default();
        for (key, columns) in iter {
            samples.set(key, Some(&columns));
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(x: i32, z: i32, level: u8) -> TileKey {
        TileKey { x, z, level }
    }

    #[test]
    fn a_change_leaves_a_batchs_copy_as_it_was() {
        let a = Arc::new(TileColumns::from_columns(&[]));
        let b = Arc::new(TileColumns::from_columns(&[]));
        let mut live: Samples = [(key(0, 0, 0), a.clone()), (key(40, -3, 1), a.clone())]
            .into_iter()
            .collect();
        let batch = live.clone();
        live.set(key(0, 0, 0), Some(&b));
        live.set(key(40, -3, 1), None);
        live.set(key(-9, 2, 0), Some(&b));
        assert!(Arc::ptr_eq(batch.get(&key(0, 0, 0)).unwrap(), &a));
        assert!(batch.get(&key(40, -3, 1)).is_some());
        assert!(batch.get(&key(-9, 2, 0)).is_none());
        assert!(Arc::ptr_eq(live.get(&key(0, 0, 0)).unwrap(), &b));
        assert!(live.get(&key(40, -3, 1)).is_none());
        live.retain(|k| k.x < 0);
        assert_eq!(
            live.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [key(-9, 2, 0)]
        );
        assert_eq!(batch.iter().count(), 2);
        // While `batch` shares a shard, `release_if_unshared` leaves it alone.
        let shared_key = key(-9, 2, 0);
        let snap = live.clone();
        live.release_if_unshared(shared_key);
        assert!(live.get(&shared_key).is_some());
        drop(snap);
        live.release_if_unshared(shared_key);
        assert!(live.get(&shared_key).is_none());
    }
}
