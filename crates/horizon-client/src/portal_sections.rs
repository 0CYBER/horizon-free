//! Memoized nether-portal palette eligibility per resident chunk.
//!
//! `Game::find_entity_portals` asks the same sections every tick whether their
//! palette may hold a portal state. The answer only changes when a chunk's
//! blocks change, so it is kept per chunk until the weak handle stops matching.
use crate::{
    chunk::Chunk,
    chunk_map::ChunkMap,
    fast_hash::{FxHashMap, FxHashSet},
    section_mask::SectionMask,
};
use std::sync::{Arc, Weak};

struct Cached {
    chunk: Weak<Chunk>,
    /// The sections whose palette may hold a portal.
    sections: SectionMask,
}

/// Resident chunks are immutable behind `Arc`; edits use `Arc::make_mut`.
/// A weak reference detects both edits and replacement, without retaining block
/// data or forcing a deep copy on the next edit. It also prevents address reuse.
/// Clear this cache whenever the gameplay facts change (including resource reload).
#[derive(Default)]
pub struct PortalSections {
    chunks: FxHashMap<(i32, i32), Cached>,
    /// The resident chunks with a portal state in a palette, as of `synced`.
    holding: FxHashSet<(i32, i32)>,
    /// The `ChunkMap::touch_cursor` `holding` was read at.
    synced: Option<(u64, u64)>,
}

impl PortalSections {
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.holding.clear();
        self.synced = None;
    }
    /// Whether any resident chunk of `chunks` has a portal state in a section's palette
    /// (what scanning every chunk with [`Self::get`] answers), read again only for the
    /// chunks touched since the last ask: none, in a level where nothing was written.
    pub fn any(&mut self, chunks: &ChunkMap, mut portal: impl FnMut(u32) -> bool) -> bool {
        let now = chunks.touch_cursor();
        if self.synced == Some(now) {
            return !self.holding.is_empty();
        }
        let touched: Option<Vec<(i32, i32)>> = self
            .synced
            .and_then(|at| chunks.touched_since(at))
            .map(|keys| keys.copied().collect());
        match touched {
            Some(keys) => {
                for key in keys {
                    self.read(chunks, key, &mut portal);
                }
            }
            None => {
                self.holding.clear();
                for (&key, chunk) in chunks.iter() {
                    if !self.get(key, chunk, &mut portal).is_empty() {
                        self.holding.insert(key);
                    }
                }
            }
        }
        self.synced = Some(now);
        !self.holding.is_empty()
    }
    fn read(&mut self, chunks: &ChunkMap, key: (i32, i32), portal: &mut impl FnMut(u32) -> bool) {
        match chunks.get(&key) {
            Some(chunk) if !self.get(key, chunk, &mut *portal).is_empty() => {
                self.holding.insert(key);
            }
            _ => {
                self.holding.remove(&key);
            }
        }
    }
    pub fn remove(&mut self, key: &(i32, i32)) {
        self.chunks.remove(key);
    }
    /// The sections of `chunk` whose palette may hold a portal, by index
    /// (`MIN_Y`'s section is 0). `portal` is only called while
    /// (re)building an entry, so an unchanged chunk scans its palette once.
    pub fn get(
        &mut self,
        key: (i32, i32),
        chunk: &Arc<Chunk>,
        mut portal: impl FnMut(u32) -> bool,
    ) -> SectionMask {
        let cached = self.chunks.entry(key).or_insert_with(|| Cached {
            chunk: Weak::new(),
            sections: SectionMask::EMPTY,
        });
        if cached.chunk.as_ptr() != Arc::as_ptr(chunk) {
            // Preserve maybe_has, including unused entries and global palettes.
            cached.sections = chunk
                .sections
                .iter()
                .enumerate()
                .filter(|(_, section)| section.blocks.maybe_has(&mut portal))
                .map(|(i, _)| i)
                .collect();
            cached.chunk = Arc::downgrade(chunk);
        }
        cached.sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{MIN_Y, SECTIONS, Section};

    #[test]
    fn edits_replacements_compaction_and_reload() {
        let mut cache = PortalSections::default();
        let mut chunk = Arc::new(Chunk::empty(0, 0));
        assert_eq!(cache.get((0, 0), &chunk, |id| id == 7), SectionMask::EMPTY);
        cache.get((0, 0), &chunk, |_| panic!("unchanged chunk rescanned"));
        let worker = chunk.clone();
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 7);
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 7),
            SectionMask::single(0)
        );
        assert_eq!(worker.get(0, MIN_Y, 0), 0);
        // Even after the last portal block is removed, its unused palette entry counts.
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 0);
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 7),
            SectionMask::single(0)
        );
        Arc::make_mut(&mut Arc::make_mut(&mut chunk).sections[0]).compact();
        assert_eq!(cache.get((0, 0), &chunk, |id| id == 7), SectionMask::EMPTY);
        Arc::make_mut(&mut chunk).sections[SECTIONS - 1] = Arc::new(Section::default());
        cache.clear(); // resource reload, with changed facts for the same state IDs
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 0),
            SectionMask::below(SECTIONS),
            "a fresh palette is rescanned for every section"
        );
        cache.remove(&(0, 0));
        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn replacement_at_same_coordinates_and_early_snapshot() {
        let mut cache = PortalSections::default();
        let mut chunk = Arc::new(Chunk::empty(-2, 3));
        let before = cache.get((-2, 3), &chunk, |id| id == 1);
        Arc::make_mut(&mut chunk).set(0, MIN_Y + 16, 0, 1);
        assert_eq!(before, SectionMask::EMPTY);
        assert_eq!(
            cache.get((-2, 3), &chunk, |id| id == 1),
            SectionMask::single(1)
        );
        let weak = Arc::downgrade(&chunk);
        drop(chunk);
        assert!(weak.upgrade().is_none()); // cache does not retain unloaded block data
        let replacement = Arc::new(Chunk::empty(-2, 3));
        assert_eq!(
            cache.get((-2, 3), &replacement, |id| id == 1),
            SectionMask::EMPTY
        );
    }

    #[test]
    fn global_palette_covers_the_whole_id_range() {
        let mut chunk = Chunk::empty(0, 0);
        for i in [0, 9, SECTIONS - 1] {
            Arc::make_mut(&mut chunk.sections[i]).blocks =
                Section::from_blocks(&std::array::from_fn(|j| (j % 300) as u32));
        }
        let chunk = Arc::new(chunk);
        let mut cache = PortalSections::default();
        // Global maybe_has considers the entire ID range, including absent states.
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 40000),
            SectionMask::from_iter([0, 9, SECTIONS - 1])
        );
    }

    /// `any` answers what scanning every resident chunk with `get` answers, after any run of
    /// insertions, replacements, removals, edits (a portal state in, then out of a palette),
    /// whole-map writes and a swapped map, asked at every step and at random pauses.
    #[test]
    fn any_is_the_scan_of_every_chunk() {
        let portal = |id: u32| id == 7;
        let scan = |chunks: &ChunkMap, cache: &mut PortalSections| {
            chunks
                .iter()
                .any(|(&key, chunk)| !cache.get(key, chunk, portal).is_empty())
        };
        let mut seed = 99u64;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let mut chunks = ChunkMap::default();
        let mut other = ChunkMap::default();
        let (mut cache, mut reference) = (PortalSections::default(), PortalSections::default());
        let mut positives = 0;
        for step in 0..20_000 {
            let key = (next(6) as i32 - 3, next(6) as i32 - 3);
            match next(9) {
                0 | 1 => {
                    chunks.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
                }
                2 => {
                    chunks.remove(&key);
                }
                3 | 4 => {
                    if let Some(chunk) = chunks.get_mut(&key) {
                        Arc::make_mut(chunk).set(1, 0, 1, 7);
                    }
                }
                5 | 6 => {
                    if let Some(chunk) = chunks.get_mut(&key) {
                        let chunk = Arc::make_mut(chunk);
                        chunk.set(1, 0, 1, 0);
                        for section in chunk.sections.iter_mut() {
                            Arc::make_mut(section).compact();
                        }
                    }
                }
                7 if next(40) == 0 => {
                    for chunk in chunks.values_mut() {
                        Arc::make_mut(chunk).set(2, 0, 2, 0);
                    }
                }
                8 if next(300) == 0 => std::mem::swap(&mut chunks, &mut other),
                _ => {}
            }
            // Not asked at every step: touches pile up between asks.
            if next(3) == 0 {
                let expected = scan(&chunks, &mut reference);
                positives += usize::from(expected);
                assert_eq!(cache.any(&chunks, portal), expected, "step {step}");
            }
        }
        assert!(
            positives > 100,
            "the run must see both answers: {positives}"
        );
        // More touches than the map keeps between two asks: read everything again.
        let _ = cache.any(&chunks, portal);
        for i in 0..20_000 {
            let key = (i % 6 - 3, i % 5 - 3);
            if let Some(chunk) = chunks.get_mut(&key) {
                Arc::make_mut(chunk).set(1, 0, 1, if i % 2 == 0 { 7 } else { 0 });
            }
        }
        assert_eq!(cache.any(&chunks, portal), scan(&chunks, &mut reference));
    }

    /// A write through `values_mut` or a `clear` right after an ask that had read every touch
    /// must be seen by the next ask: a portal state put into a palette that way, and a map
    /// emptied, are both answered as the scan answers.
    #[test]
    fn any_sees_values_mut_and_clear_after_a_caught_up_ask() {
        let portal = |id: u32| id == 7;
        let mut chunks = ChunkMap::default();
        let mut cache = PortalSections::default();
        for key in [(0, 0), (1, 0)] {
            chunks.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        }
        assert!(!cache.any(&chunks, portal));
        // The ask above has read the whole log; the write below adds nothing to it.
        for chunk in chunks.values_mut() {
            Arc::make_mut(chunk).set(1, 0, 1, 7);
        }
        assert!(
            cache.any(&chunks, portal),
            "values_mut written portal state missed"
        );
        chunks.clear();
        assert!(
            !cache.any(&chunks, portal),
            "a cleared map still answers yes"
        );
    }
}
