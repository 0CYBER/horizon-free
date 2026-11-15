//! A level's capability generations: a number per block position that a provider advances
//! (`level_invalidate_capabilities`) when its answer there changes with no block change, which a
//! kept capability (`BlockCapabilityCache`) compares to know it must look again (arismoko/horizon#3446).
//!
//! Only positions a provider invalidated have an entry; every other position reads the level's
//! floor. Entries go with their chunk. Every number, an entry's or the floor's, is drawn once
//! from the level's own counter, so a position never reads a number it read before while the
//! level is open: when a chunk that held entries unloads, the floor moves on, and a position of
//! that chunk read again after it reloads differs from anything read before. That costs the
//! level's other uninvalidated positions one changed read, so their caches look again once.
use std::collections::HashMap;

/// The capability generations of one level, dropped with it.
#[derive(Debug, Default)]
pub struct CapabilityGenerations {
    /// The last number drawn.
    drawn: u64,
    /// What a position without an entry reads.
    floor: u64,
    /// The positions invalidated since their chunk loaded, by chunk.
    chunks: HashMap<(i32, i32), HashMap<(i32, i32, i32), u64>>,
}

fn chunk_of((x, _, z): (i32, i32, i32)) -> (i32, i32) {
    (x >> 4, z >> 4)
}

impl CapabilityGenerations {
    /// The generation at `pos`: two map lookups, no allocation.
    pub fn get(&self, pos: (i32, i32, i32)) -> u64 {
        self.chunks
            .get(&chunk_of(pos))
            .and_then(|chunk| chunk.get(&pos))
            .copied()
            .unwrap_or(self.floor)
    }
    /// Advances the generation at `pos` to a number not read before.
    pub fn invalidate(&mut self, pos: (i32, i32, i32)) {
        let next = self.draw();
        self.chunks
            .entry(chunk_of(pos))
            .or_default()
            .insert(pos, next);
    }
    /// Drops the entries of chunk `(x, z)`, which leaves the level; when it had any, the floor
    /// moves on, so none of its positions reads an earlier number again.
    pub fn unload_chunk(&mut self, chunk: (i32, i32)) {
        if self.chunks.remove(&chunk).is_some() {
            self.floor = self.draw();
        }
    }
    /// How many positions have an entry.
    pub fn len(&self) -> usize {
        self.chunks.values().map(HashMap::len).sum()
    }
    /// Whether no position has an entry.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    fn draw(&mut self) -> u64 {
        self.drawn = self
            .drawn
            .checked_add(1)
            .expect("capability generation space exhausted");
        self.drawn
    }
}

#[cfg(test)]
mod tests {
    use super::CapabilityGenerations;

    #[test]
    fn an_invalidation_changes_only_its_own_position() {
        let mut generations = CapabilityGenerations::default();
        let (pipe, machine) = ((3, 64, 3), (4, 64, 3));
        let (pipe_was, machine_was) = (generations.get(pipe), generations.get(machine));
        generations.invalidate(machine);
        assert_eq!(generations.get(pipe), pipe_was);
        assert_ne!(generations.get(machine), machine_was);
        let once = generations.get(machine);
        generations.invalidate(machine);
        assert_ne!(generations.get(machine), once);
        assert_ne!(generations.get(machine), machine_was);
        assert_eq!(generations.len(), 1);
    }

    #[test]
    fn a_position_never_reads_an_earlier_number_across_its_chunk_unloading() {
        let mut generations = CapabilityGenerations::default();
        let (invalidated, untouched) = ((17, 70, -5), (20, 70, -1));
        let mut read = vec![generations.get(invalidated), generations.get(untouched)];
        generations.invalidate(invalidated);
        read.push(generations.get(invalidated));
        generations.unload_chunk((1, -1));
        assert!(generations.is_empty());
        for pos in [invalidated, untouched] {
            assert!(!read.contains(&generations.get(pos)), "{pos:?} read again");
        }
    }

    #[test]
    fn a_chunk_without_entries_unloads_without_changing_any_read() {
        let mut generations = CapabilityGenerations::default();
        let (kept, elsewhere) = ((0, 64, 0), (100, 64, 100));
        generations.invalidate(kept);
        let (kept_was, elsewhere_was) = (generations.get(kept), generations.get(elsewhere));
        generations.unload_chunk((6, 6));
        assert_eq!(generations.get(kept), kept_was);
        assert_eq!(generations.get(elsewhere), elsewhere_was);
    }
}
