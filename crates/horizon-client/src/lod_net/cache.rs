//! The server's far tiles, kept for every player: a tile is the same for
//! everyone in its level, so players looking at the same land cost one
//! sample. A tile is kept as [`super::encode_tile`] makes it, without a
//! request's id, so a kept tile answers each request with the bytes a fresh
//! sample would ([`super::reply`]): a refusal too, being the same for every
//! request.
//!
//! Tiles are kept by level, position, stride and detail, under the level's
//! far terrain source: a level opened again (another source: its generator
//! and saves read anew) drops its tiles. The oldest used go first past
//! [`MAX_BYTES`].
use super::{Request, reply};
use crate::streaming::LodSource;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Weak},
};

/// What the kept tiles may weigh, replies' bytes: about 4,000 tiles of a
/// mean 8 KiB (`docs/hosting.md`).
pub const MAX_BYTES: usize = 32 << 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    level: u32,
    x: i32,
    z: i32,
    stride: i32,
    approximate: bool,
}

struct Kept {
    body: Arc<[u8]>,
    used: u64,
}

/// A level's tiles: its name and the source they were sampled from. A
/// `Weak` keeps the source's address from being taken by another while
/// tiles of it are kept, without keeping its generator alive.
struct Level {
    name: String,
    source: Weak<dyn LodSource>,
}

#[derive(Default)]
pub struct TileCache {
    levels: Vec<Level>,
    tiles: HashMap<Key, Kept>,
    /// Keys by last use, oldest first.
    order: BTreeMap<u64, Key>,
    clock: u64,
    bytes: usize,
    max_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

impl TileCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    /// The level's index for tiles of `source`; a level seen before under
    /// another source loses its tiles.
    fn level(&mut self, name: &str, source: &Arc<dyn LodSource>) -> u32 {
        let same = |level: &Level| std::ptr::addr_eq(level.source.as_ptr(), Arc::as_ptr(source));
        if let Some(at) = self.levels.iter().position(|l| l.name == name) {
            if same(&self.levels[at]) {
                return at as u32;
            }
            self.forget(at as u32);
            self.levels[at].source = Arc::downgrade(source);
            return at as u32;
        }
        self.levels.push(Level {
            name: name.to_owned(),
            source: Arc::downgrade(source),
        });
        (self.levels.len() - 1) as u32
    }

    fn forget(&mut self, level: u32) {
        let gone: Vec<Key> = self
            .tiles
            .keys()
            .filter(|k| k.level == level)
            .copied()
            .collect();
        for key in gone {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &Key) {
        if let Some(kept) = self.tiles.remove(key) {
            self.order.remove(&kept.used);
            self.bytes -= kept.body.len();
        }
    }

    fn key(&mut self, level: &str, source: &Arc<dyn LodSource>, r: &Request) -> Key {
        Key {
            level: self.level(level, source),
            x: r.x,
            z: r.z,
            stride: r.stride,
            approximate: r.approximate,
        }
    }

    /// The reply to `request` from a kept tile.
    pub fn get(
        &mut self,
        level: &str,
        source: &Arc<dyn LodSource>,
        request: &Request,
    ) -> Option<Vec<u8>> {
        let key = self.key(level, source, request);
        let Some(kept) = self.tiles.get_mut(&key) else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        self.clock += 1;
        self.order.remove(&kept.used);
        kept.used = self.clock;
        self.order.insert(self.clock, key);
        Some(reply(request.id, &kept.body))
    }

    /// Keep `tile` ([`super::encode_tile`]), sampled for `request`.
    pub fn put(
        &mut self,
        level: &str,
        source: &Arc<dyn LodSource>,
        request: &Request,
        tile: &[u8],
    ) {
        if tile.len() > self.max_bytes / 4 {
            return;
        }
        let key = self.key(level, source, request);
        self.remove(&key);
        self.clock += 1;
        self.bytes += tile.len();
        self.tiles.insert(
            key,
            Kept {
                body: tile.into(),
                used: self.clock,
            },
        );
        self.order.insert(self.clock, key);
        while self.bytes > self.max_bytes
            && let Some((_, oldest)) = self.order.pop_first()
        {
            if let Some(kept) = self.tiles.remove(&oldest) {
                self.bytes -= kept.body.len();
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.tiles.len()
    }
    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lod_net::{MAX_REPLY, refusal},
        streaming::LodSampler,
    };

    struct Flat;
    impl LodSampler for Flat {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            None
        }
    }
    impl LodSource for Flat {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Flat)
        }
    }

    fn request(id: u32, x: i32) -> Request {
        Request {
            id,
            x,
            z: 0,
            stride: 1,
            approximate: false,
        }
    }

    #[test]
    fn a_kept_reply_a_longer_id_takes_past_the_limit_is_refused() {
        let source: Arc<dyn LodSource> = Arc::new(Flat);
        let mut cache = TileCache::new(4 * MAX_REPLY);
        // As long as a tile may be under a one-byte id.
        let mut reply = vec![0];
        reply.resize(MAX_REPLY, 7);
        cache.put("o", &source, &request(0, 0), &reply[1..]);
        reply[0] = 5;
        assert_eq!(cache.get("o", &source, &request(5, 0)), Some(reply));
        let long = 1 << 20;
        assert_eq!(
            cache.get("o", &source, &request(long, 0)),
            Some(refusal(long))
        );
    }

    #[test]
    fn the_oldest_used_tiles_go_first_and_a_new_source_drops_its_level() {
        let source: Arc<dyn LodSource> = Arc::new(Flat);
        let mut cache = TileCache::new(4 * 100);
        let tile = [7; 100];
        let reply = |id: u32| reply(id, &tile);
        for (i, x) in [0, 16, 32, 48].into_iter().enumerate() {
            cache.put("o", &source, &request(i as u32, x), &tile);
        }
        assert_eq!(cache.bytes(), 400);
        // Used again: the second oldest goes instead.
        assert_eq!(cache.get("o", &source, &request(300, 0)), Some(reply(300)));
        cache.put("o", &source, &request(9, 64), &tile);
        assert_eq!(cache.len(), 4);
        assert!(cache.get("o", &source, &request(1, 16)).is_none());
        assert!(cache.get("o", &source, &request(1, 0)).is_some());
        // Another level keeps its own; the level opened again loses its tiles.
        cache.put("n", &source, &request(1, 0), &tile);
        let reopened: Arc<dyn LodSource> = Arc::new(Flat);
        assert!(cache.get("o", &reopened, &request(1, 0)).is_none());
        assert!(cache.get("n", &source, &request(1, 0)).is_some());
    }
}
