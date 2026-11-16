//! `RandomSequences`: the named random streams `/random` draws from.
use horizon_core::random::{XoroshiroRandom, mix_stafford13, seed_from_hash};
use std::collections::BTreeMap;

const GOLDEN_RATIO: u64 = 0x9e37_79b9_7f4a_7c15;
const SILVER_RATIO: u64 = 0x6a09_e667_f3bc_c909;

#[derive(Clone, Debug)]
pub struct RandomSequences {
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
    sequences: BTreeMap<String, XoroshiroRandom>,
    /// `SavedData.dirty`: a draw marks it (`DirtyMarkingRandomSource`), the save clears
    /// it (`SavedDataStorage.collectDirtyTagsToSave`); `clear`, `setSeedDefaults` and
    /// `reset` leave it alone.
    dirty: bool,
    /// The sequence `get` handed out last, as it was then: it was drawn from if it moved
    /// (a draw always moves a xoroshiro stream), the moment vanilla's wrapper marks the data.
    watched: Option<(String, XoroshiroRandom)>,
    /// Sequences out on `lend`, as they left: still the world's, so the save keeps them.
    lent: BTreeMap<String, XoroshiroRandom>,
}
impl Default for RandomSequences {
    fn default() -> Self {
        Self {
            salt: 0,
            include_world_seed: true,
            include_sequence_id: true,
            sequences: BTreeMap::new(),
            dirty: false,
            watched: None,
            lent: BTreeMap::new(),
        }
    }
}
/// `RandomSequence.createSequence`.
fn create(id: &str, world_seed: i64, salt: i32, world: bool, with_id: bool) -> XoroshiroRandom {
    let seed = (if world { world_seed } else { 0 }) ^ i64::from(salt);
    let mut lo = seed as u64 ^ SILVER_RATIO;
    let mut hi = lo.wrapping_add(GOLDEN_RATIO);
    if with_id {
        let [a, b] = seed_from_hash(id);
        lo ^= a;
        hi ^= b;
    }
    XoroshiroRandom::from_state(mix_stafford13(lo), mix_stafford13(hi))
}
impl RandomSequences {
    /// `RandomSequences.get`: the caller's draws on it mark the data dirty.
    pub fn get(&mut self, id: &str, world_seed: i64) -> &mut XoroshiroRandom {
        self.settle();
        let (salt, w, i) = (self.salt, self.include_world_seed, self.include_sequence_id);
        let random = self
            .sequences
            .entry(id.to_owned())
            .or_insert_with(|| create(id, world_seed, salt, w, i));
        self.watched = Some((id.to_owned(), random.clone()));
        random
    }
    /// Whether the sequence `get` handed out last was drawn from.
    fn drew(&self) -> bool {
        self.watched
            .as_ref()
            .is_some_and(|(id, then)| self.sequences.get(id).is_some_and(|now| now != then))
    }
    /// Marks the data for a draw on the sequence `get` handed out last, which nothing
    /// can draw from any more once another method runs.
    fn settle(&mut self) {
        self.dirty |= self.drew();
        self.watched = None;
    }
    /// New data for a world with no `random_sequences.dat`: `MinecraftServer`'s
    /// `savedDataStorage.computeIfAbsent(RandomSequences.TYPE)` builds it and
    /// `SavedDataStorage.set` marks it dirty, so the world's first save writes it.
    pub fn new_dirty() -> Self {
        Self {
            dirty: true,
            ..Self::default()
        }
    }
    /// `SavedDataStorage.collectDirtyTagsToSave`: the data to write when it is dirty,
    /// which `setDirty(false)` then clears.
    pub fn take_dirty_nbt(&mut self) -> Option<horizon_core::nbt::Tag> {
        self.settle();
        std::mem::take(&mut self.dirty).then(|| self.to_nbt())
    }
    /// Lends sequence `id` out (created if missing); `give_back` returns it. Lending
    /// is no draw; what was drawn from it marks the data when it comes back.
    pub fn lend(&mut self, id: &str, world_seed: i64) -> XoroshiroRandom {
        self.settle();
        let (salt, w, i) = (self.salt, self.include_world_seed, self.include_sequence_id);
        let random = self
            .sequences
            .remove(id)
            .unwrap_or_else(|| create(id, world_seed, salt, w, i));
        self.lent.insert(id.to_owned(), random.clone());
        random
    }
    pub fn give_back(&mut self, id: String, random: XoroshiroRandom) {
        self.settle();
        if self.lent.remove(&id).is_none_or(|then| then != random) {
            self.dirty = true;
        }
        self.sequences.insert(id, random);
    }
    /// `RandomSequences.CODEC` (`random_sequences.dat`'s data).
    pub fn to_nbt(&self) -> horizon_core::nbt::Tag {
        use horizon_core::nbt::Tag;
        let mut all: BTreeMap<&str, &XoroshiroRandom> =
            self.lent.iter().map(|(id, r)| (id.as_str(), r)).collect();
        all.extend(self.sequences.iter().map(|(id, r)| (id.as_str(), r)));
        let sequences = all
            .into_iter()
            .map(|(id, r)| {
                let (lo, hi) = r.state_words();
                (
                    id,
                    Tag::compound([("source", Tag::LongArray(vec![lo as i64, hi as i64]))]),
                )
            })
            .collect::<Vec<_>>();
        Tag::compound([
            ("salt", Tag::Int(self.salt)),
            (
                "include_world_seed",
                Tag::Byte(i8::from(self.include_world_seed)),
            ),
            (
                "include_sequence_id",
                Tag::Byte(i8::from(self.include_sequence_id)),
            ),
            ("sequences", Tag::compound(sequences)),
        ])
    }
    pub fn from_nbt(tag: &horizon_core::nbt::Tag) -> Self {
        use horizon_core::nbt::Tag;
        let int = |k: &str| match tag.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let mut out = Self {
            salt: int("salt").unwrap_or(0),
            include_world_seed: int("include_world_seed").unwrap_or(1) != 0,
            include_sequence_id: int("include_sequence_id").unwrap_or(1) != 0,
            sequences: BTreeMap::new(),
            dirty: false,
            watched: None,
            lent: BTreeMap::new(),
        };
        if let Some(Tag::Compound(c)) = tag.get("sequences") {
            for (id, v) in c {
                let words = match v.get("source") {
                    Some(Tag::LongArray(a)) if a.len() == 2 => Some((a[0] as u64, a[1] as u64)),
                    Some(Tag::List(l)) if l.len() == 2 => match (&l[0], &l[1]) {
                        (Tag::Long(a), Tag::Long(b)) => Some((*a as u64, *b as u64)),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((lo, hi)) = words {
                    out.sequences
                        .insert(id.to_string_lossy(), XoroshiroRandom::from_state(lo, hi));
                }
            }
        }
        out
    }
    pub fn ids(&self) -> Vec<String> {
        self.sequences.keys().cloned().collect()
    }
    pub fn clear(&mut self) -> usize {
        self.settle();
        let n = self.sequences.len();
        self.sequences.clear();
        n
    }
    pub fn set_defaults(&mut self, salt: i32, include_world_seed: bool, include_sequence_id: bool) {
        self.salt = salt;
        self.include_world_seed = include_world_seed;
        self.include_sequence_id = include_sequence_id;
    }
    pub fn reset(&mut self, id: &str, world_seed: i64) {
        self.settle();
        let (salt, w, i) = (self.salt, self.include_world_seed, self.include_sequence_id);
        self.sequences
            .insert(id.to_owned(), create(id, world_seed, salt, w, i));
    }
    pub fn reset_with(&mut self, id: &str, world_seed: i64, salt: i32, w: bool, i: bool) {
        self.settle();
        self.sequences
            .insert(id.to_owned(), create(id, world_seed, salt, w, i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::RandomSource;

    #[test]
    fn sequences_are_deterministic_and_resettable() {
        let mut a = RandomSequences::default();
        let first: Vec<i32> = (0..4)
            .map(|_| a.get("minecraft:x", 42).next_int_bound(100).unwrap())
            .collect();
        let mut b = RandomSequences::default();
        let again: Vec<i32> = (0..4)
            .map(|_| b.get("minecraft:x", 42).next_int_bound(100).unwrap())
            .collect();
        assert_eq!(first, again);
        // Another id or seed gives another stream.
        let other = RandomSequences::default()
            .get("minecraft:y", 42)
            .next_long();
        let same = RandomSequences::default()
            .get("minecraft:x", 42)
            .next_long();
        assert_ne!(other, same);
        // Reset restarts it.
        a.reset("minecraft:x", 42);
        assert_eq!(
            a.get("minecraft:x", 42).next_int_bound(100).unwrap(),
            first[0]
        );
        // Defaults change what new sequences start from.
        a.set_defaults(7, false, false);
        a.reset("minecraft:x", 42);
        let v = a.get("minecraft:x", 1).next_long();
        let mut c = RandomSequences::default();
        c.set_defaults(7, false, false);
        assert_eq!(c.get("minecraft:z", 999).next_long(), v);
        assert_eq!(a.clear(), 1);
    }
}
