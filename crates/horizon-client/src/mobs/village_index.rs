//! `PoiManager.sectionsToVillage` over the loop's snapshot: the Chebyshev distance in sections
//! from a section to one holding a village point of interest.
//!
//! [`village_sections`](super::villager_brain::village_sections) read every villager of the
//! snapshot, and the block under each of their sites, for every mob that asked. The sections of
//! the villagers' sites are the same for every mob until a villager's entry of the snapshot
//! changes (its turn ended, or it left the level); the blocks are the level's, which the loop
//! only reads (writes land between its passes, where the loop starts the index again). So the
//! index is built when a mob first asks, and each villager entry the loop writes afterwards is
//! marked ([`VillageIndex::mark`]) and read again at the next question.
use super::{MobWorld, Seen, Species};
use crate::fast_hash::FxHashMap;
use std::sync::{Arc, Mutex, PoisonError};

type Section = [i32; 3];

/// How far `sections_to_village` looks, in sections.
const REACH: i32 = 7;
/// Sections per side of a bucket of the lookup.
const BUCKET_SHIFT: i32 = 3;

/// One villager entry's sections: its four sites at most.
#[derive(Clone, Copy, Default)]
struct Held {
    len: u8,
    sections: [Section; 4],
}

#[derive(Clone, Default)]
struct Sections {
    built: bool,
    /// Entries of the snapshot read again at the next question.
    dirty: Vec<u32>,
    /// What each entry of the snapshot holds in the buckets.
    held: Vec<Held>,
    /// Each section some villager holds, with how many hold it, by bucket.
    buckets: FxHashMap<Section, Vec<(Section, u32)>>,
}

/// The villagers' sections of one loop's snapshot, shared by its mobs.
#[derive(Default)]
pub struct VillageIndex {
    state: Mutex<Arc<Sections>>,
}

fn bucket(s: Section) -> Section {
    s.map(|v| v >> BUCKET_SHIFT)
}

impl Sections {
    fn add(&mut self, s: Section) {
        let list = self.buckets.entry(bucket(s)).or_default();
        match list.iter_mut().find(|(v, _)| *v == s) {
            Some((_, n)) => *n += 1,
            None => list.push((s, 1)),
        }
    }
    fn remove(&mut self, s: Section) {
        let key = bucket(s);
        if let Some(list) = self.buckets.get_mut(&key)
            && let Some(i) = list.iter().position(|(v, _)| *v == s)
        {
            list[i].1 -= 1;
            if list[i].1 == 0 {
                list.swap_remove(i);
                if list.is_empty() {
                    self.buckets.remove(&key);
                }
            }
        }
    }
    /// Slot `slot`'s entry as the snapshot has it now.
    fn read(&mut self, slot: usize, w: &MobWorld) {
        if self.held.len() <= slot {
            self.held.resize(slot + 1, Held::default());
        }
        let old = std::mem::take(&mut self.held[slot]);
        for &s in &old.sections[..usize::from(old.len)] {
            self.remove(s);
        }
        let Some(o) = w.others.get(slot) else { return };
        let mut now = Held::default();
        if o.alive && o.species == Species::Villager {
            for p in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                if super::villager_brain::poi_at(w, p[0], p[1], p[2]).is_some() {
                    let s = [p[0] >> 4, p[1] >> 4, p[2] >> 4];
                    if !now.sections[..usize::from(now.len)].contains(&s) {
                        now.sections[usize::from(now.len)] = s;
                        now.len += 1;
                    }
                }
            }
        }
        for &s in &now.sections[..usize::from(now.len)] {
            self.add(s);
        }
        self.held[slot] = now;
    }
    /// Brings the buckets to the snapshot `w` holds.
    fn sync(&mut self, w: &MobWorld) {
        if !self.built {
            self.built = true;
            self.dirty.clear();
            self.held.clear();
            self.buckets.clear();
            self.held.resize(w.others.len(), Held::default());
            match w.others_villagers {
                Some(slots) => {
                    for &i in slots {
                        self.read(i as usize, w);
                    }
                }
                None => {
                    for i in 0..w.others.len() {
                        if w.others[i].species == Species::Villager {
                            self.read(i, w);
                        }
                    }
                }
            }
            return;
        }
        for slot in std::mem::take(&mut self.dirty) {
            self.read(slot as usize, w);
        }
    }
    /// `PoiManager.sectionsToVillage`'s distance from `at` to the sections held, at most `REACH`.
    fn distance(&self, at: Section) -> i32 {
        let mut best = REACH;
        let lo = at.map(|v| (v - REACH) >> BUCKET_SHIFT);
        let hi = at.map(|v| (v + REACH) >> BUCKET_SHIFT);
        for bx in lo[0]..=hi[0] {
            for by in lo[1]..=hi[1] {
                for bz in lo[2]..=hi[2] {
                    let Some(list) = self.buckets.get(&[bx, by, bz]) else {
                        continue;
                    };
                    for (v, _) in list {
                        let d = (v[0] - at[0])
                            .abs()
                            .max((v[1] - at[1]).abs())
                            .max((v[2] - at[2]).abs());
                        best = best.min(d);
                    }
                }
            }
        }
        best
    }
}

impl VillageIndex {
    /// Entry `slot` of the snapshot changed (or `seen` is about to replace it): read it again at
    /// the next question. Call it for every write to a villager's entry.
    pub fn mark(&mut self, slot: usize, was: &Seen, now: &Seen) {
        if was.species != Species::Villager && now.species != Species::Villager {
            return;
        }
        let state = Arc::make_mut(self.state.get_mut().unwrap_or_else(PoisonError::into_inner));
        if state.built {
            state.dirty.push(slot as u32);
        }
    }
    /// The index at the snapshot `w` holds.
    fn at(&self, w: &MobWorld) -> Arc<Sections> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.built || !state.dirty.is_empty() {
            Arc::make_mut(&mut state).sync(w);
        }
        Arc::clone(&state)
    }
}

/// The village sections a mob sees: the villagers' of the snapshot and its own.
pub struct Villages {
    shared: Arc<Sections>,
    own: Held,
}

impl Villages {
    pub(super) fn new(w: &MobWorld, own: &[Section]) -> Self {
        let shared = match w.village {
            Some(index) => index.at(w),
            _ => {
                let mut local = Sections::default();
                local.sync(w);
                Arc::new(local)
            }
        };
        let mut held = Held::default();
        for &s in own {
            if !held.sections[..usize::from(held.len)].contains(&s) {
                held.sections[usize::from(held.len)] = s;
                held.len += 1;
            }
        }
        Self { shared, own: held }
    }
    /// `PoiManager.sectionsToVillage`: the Chebyshev distance in sections to a village section,
    /// at most 7.
    pub fn distance(&self, s: Section) -> i32 {
        let mut best = self.shared.distance(s);
        for v in &self.own.sections[..usize::from(self.own.len)] {
            let d = (v[0] - s[0])
                .abs()
                .max((v[1] - s[1]).abs())
                .max((v[2] - s[2]).abs());
            best = best.min(d);
        }
        best
    }
}

#[cfg(test)]
#[path = "village_index_tests.rs"]
mod tests;
