//! Villager gossip (`GossipContainer` and `GossipType`): what a villager
//! remembers about each player, how it fades by the day, how it spreads from
//! villager to villager and the reputation it adds up to.
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::nbt::Tag;
use std::collections::BTreeMap;

/// `ReputationEventType` amounts.
pub const REPUTATION_CHANGE_PER_EVENT: i32 = 25;
pub const REPUTATION_CHANGE_PER_TRADE: i32 = 2;
/// `GossipContainer.DISCARD_THRESHOLD`: values below this are forgotten.
const DISCARD_THRESHOLD: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GossipType {
    MajorNegative,
    MinorNegative,
    MinorPositive,
    MajorPositive,
    Trading,
}
impl GossipType {
    pub const ALL: [Self; 5] = [
        Self::MajorNegative,
        Self::MinorNegative,
        Self::MinorPositive,
        Self::MajorPositive,
        Self::Trading,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::MajorNegative => "major_negative",
            Self::MinorNegative => "minor_negative",
            Self::MinorPositive => "minor_positive",
            Self::MajorPositive => "major_positive",
            Self::Trading => "trading",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
    fn index(self) -> usize {
        self as usize
    }
    pub fn weight(self) -> i32 {
        match self {
            Self::MajorNegative => -5,
            Self::MinorNegative => -1,
            Self::MinorPositive => 1,
            Self::MajorPositive => 5,
            Self::Trading => 1,
        }
    }
    pub fn max(self) -> i32 {
        match self {
            Self::MajorNegative => 100,
            Self::MinorNegative => 200,
            Self::MinorPositive | Self::Trading => 25,
            Self::MajorPositive => 20,
        }
    }
    pub fn decay_per_day(self) -> i32 {
        match self {
            Self::MajorNegative => 10,
            Self::MinorNegative => 20,
            Self::MinorPositive => 1,
            Self::MajorPositive => 0,
            Self::Trading => 2,
        }
    }
    pub fn decay_per_transfer(self) -> i32 {
        match self {
            Self::MajorNegative => 10,
            Self::MinorNegative => 20,
            Self::MinorPositive => 5,
            Self::MajorPositive | Self::Trading => 20,
        }
    }
}

/// A UUID as four ints.
pub type Target = [i32; 4];

/// One gossip entry (`GossipContainer.GossipEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    target: Target,
    kind: GossipType,
    value: i32,
}
impl Entry {
    fn weighted(self) -> i32 {
        self.value * self.kind.weight()
    }
}

/// `GossipContainer`: per target, the value of each gossip type (0: none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gossips {
    map: BTreeMap<Target, [i32; 5]>,
}
impl Gossips {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn clear(&mut self) {
        self.map.clear();
    }
    /// Every (target, type, value) held.
    pub fn entries(&self) -> impl Iterator<Item = (Target, GossipType, i32)> + '_ {
        self.map.iter().flat_map(|(t, e)| {
            GossipType::ALL
                .into_iter()
                .filter(move |k| e[k.index()] != 0)
                .map(move |k| (*t, k, e[k.index()]))
        })
    }
    pub fn get(&self, target: Target, kind: GossipType) -> i32 {
        self.map.get(&target).map_or(0, |e| e[kind.index()])
    }
    /// `getReputation`: the weighted sum of every type for a target.
    pub fn reputation(&self, target: Target) -> i32 {
        self.map.get(&target).map_or(0, |e| {
            GossipType::ALL
                .into_iter()
                .map(|t| e[t.index()] * t.weight())
                .sum()
        })
    }
    /// How many targets have a gossip of `kind` whose weighted value passes `test`.
    pub fn count_for_type(&self, kind: GossipType, test: impl Fn(f64) -> bool) -> usize {
        self.map
            .values()
            .filter(|e| test(f64::from(e[kind.index()] * kind.weight())))
            .count()
    }
    /// `add`: sums up to the type's maximum and forgets values below 2.
    pub fn add(&mut self, target: Target, kind: GossipType, amount: i32) {
        let entries = self.map.entry(target).or_insert([0; 5]);
        let old = entries[kind.index()];
        entries[kind.index()] = if old == 0 {
            amount
        } else {
            let sum = old + amount;
            if sum > kind.max() {
                kind.max().max(old)
            } else {
                sum
            }
        };
        let value = entries[kind.index()];
        if value > kind.max() {
            entries[kind.index()] = kind.max();
        }
        if value < DISCARD_THRESHOLD {
            entries[kind.index()] = 0;
        }
        if entries.iter().all(|v| *v == 0) {
            self.map.remove(&target);
        }
    }
    pub fn remove(&mut self, target: Target, kind: GossipType) {
        if let Some(e) = self.map.get_mut(&target) {
            e[kind.index()] = 0;
            if e.iter().all(|v| *v == 0) {
                self.map.remove(&target);
            }
        }
    }
    /// `decay` (once per day): every type loses its daily amount.
    pub fn decay(&mut self) {
        self.map.retain(|_, e| {
            for kind in GossipType::ALL {
                let v = e[kind.index()];
                if v == 0 {
                    continue;
                }
                let next = v - kind.decay_per_day();
                e[kind.index()] = if next < DISCARD_THRESHOLD { 0 } else { next };
            }
            e.iter().any(|v| *v != 0)
        });
    }
    fn unpack(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for (target, e) in &self.map {
            for kind in GossipType::ALL {
                if e[kind.index()] != 0 {
                    out.push(Entry {
                        target: *target,
                        kind,
                        value: e[kind.index()],
                    });
                }
            }
        }
        out
    }
    /// `selectGossipsForTransfer`: `max_count` picks weighted by magnitude,
    /// duplicates counted once.
    fn select_for_transfer(&self, random: &mut LegacyRandom, max_count: usize) -> Vec<Entry> {
        let entries = self.unpack();
        if entries.is_empty() {
            return Vec::new();
        }
        let mut ends = Vec::with_capacity(entries.len());
        let mut total = 0;
        for e in &entries {
            total += e.weighted().abs();
            ends.push(total - 1);
        }
        let mut picked: Vec<usize> = Vec::new();
        for _ in 0..max_count {
            let choice = random.next_int_bound(total).unwrap_or(0);
            // Arrays.binarySearch on the strictly increasing range ends.
            let index = match ends.binary_search(&choice) {
                Ok(i) | Err(i) => i,
            };
            if !picked.contains(&index) {
                picked.push(index);
            }
        }
        picked.into_iter().map(|i| entries[i]).collect()
    }
    /// `transferFrom`: gossip heard from another villager, weakened by the
    /// retelling. Returns how many gossips were picked.
    pub fn transfer_from(
        &mut self,
        source: &Gossips,
        random: &mut LegacyRandom,
        max_count: usize,
    ) -> usize {
        let picked = source.select_for_transfer(random, max_count);
        for g in &picked {
            let decayed = g.value - g.kind.decay_per_transfer();
            if decayed >= DISCARD_THRESHOLD {
                let e = self.map.entry(g.target).or_insert([0; 5]);
                let slot = &mut e[g.kind.index()];
                *slot = (*slot).max(decayed);
            }
        }
        picked.len()
    }

    pub fn to_nbt(&self) -> Tag {
        Tag::List(
            self.unpack()
                .into_iter()
                .map(|e| {
                    Tag::compound([
                        ("Target", Tag::IntArray(e.target.to_vec())),
                        ("Type", Tag::String(e.kind.id().into())),
                        ("Value", Tag::Int(e.value)),
                    ])
                })
                .collect(),
        )
    }
    pub fn from_nbt(tag: &Tag) -> Self {
        let mut out = Self::default();
        let Tag::List(list) = tag else { return out };
        for g in list {
            let target = match g.get("Target") {
                Some(Tag::IntArray(v)) if v.len() == 4 => [v[0], v[1], v[2], v[3]],
                _ => continue,
            };
            let kind = match g.get("Type") {
                Some(Tag::String(s)) => GossipType::from_id(&s.to_string_lossy()),
                _ => None,
            };
            let value = match g.get("Value") {
                Some(Tag::Int(v)) => *v,
                _ => continue,
            };
            if let Some(kind) = kind
                && value > 0
            {
                out.map.entry(target).or_insert([0; 5])[kind.index()] = value;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: Target = [1, 2, 3, 4];
    const OTHER: Target = [5, 6, 7, 8];

    #[test]
    fn reputation_adds_up_weighted() {
        let mut g = Gossips::default();
        g.add(PLAYER, GossipType::MinorPositive, 25);
        g.add(PLAYER, GossipType::MajorPositive, 20);
        g.add(PLAYER, GossipType::Trading, 2);
        // 25 + 20 * 5 + 2
        assert_eq!(g.reputation(PLAYER), 127);
        g.add(PLAYER, GossipType::MajorNegative, 25);
        assert_eq!(g.reputation(PLAYER), 127 - 125);
        assert_eq!(g.reputation(OTHER), 0);
    }
    #[test]
    fn values_stop_at_their_maximum() {
        let mut g = Gossips::default();
        for _ in 0..10 {
            g.add(PLAYER, GossipType::MajorNegative, 25);
        }
        assert_eq!(g.get(PLAYER, GossipType::MajorNegative), 100);
        // The minor negative cap is 200: eight hits of 25 fill it.
        for _ in 0..12 {
            g.add(PLAYER, GossipType::MinorNegative, 25);
        }
        assert_eq!(g.get(PLAYER, GossipType::MinorNegative), 200);
        assert_eq!(g.reputation(PLAYER), -500 - 200);
    }
    #[test]
    fn small_values_are_forgotten() {
        let mut g = Gossips::default();
        g.add(PLAYER, GossipType::Trading, 1);
        assert!(g.is_empty());
        g.add(PLAYER, GossipType::Trading, 2);
        assert_eq!(g.get(PLAYER, GossipType::Trading), 2);
        g.remove(PLAYER, GossipType::Trading);
        assert!(g.is_empty());
    }
    #[test]
    fn gossip_decays_by_the_day() {
        let mut g = Gossips::default();
        g.add(PLAYER, GossipType::MajorNegative, 25);
        g.add(PLAYER, GossipType::MinorNegative, 25);
        g.add(PLAYER, GossipType::MinorPositive, 3);
        g.add(PLAYER, GossipType::MajorPositive, 20);
        g.decay();
        assert_eq!(g.get(PLAYER, GossipType::MajorNegative), 15);
        assert_eq!(g.get(PLAYER, GossipType::MinorNegative), 5);
        assert_eq!(g.get(PLAYER, GossipType::MinorPositive), 2);
        // Major positive gossip never fades.
        assert_eq!(g.get(PLAYER, GossipType::MajorPositive), 20);
        g.decay();
        assert_eq!(g.get(PLAYER, GossipType::MajorNegative), 5);
        assert_eq!(g.get(PLAYER, GossipType::MinorNegative), 0);
        assert_eq!(g.get(PLAYER, GossipType::MinorPositive), 0);
        g.decay();
        assert_eq!(g.get(PLAYER, GossipType::MajorNegative), 0);
        assert_eq!(g.len(), 1);
    }
    #[test]
    fn retold_gossip_is_weaker() {
        let mut source = Gossips::default();
        source.add(PLAYER, GossipType::MajorNegative, 25);
        source.add(OTHER, GossipType::Trading, 25);
        let mut heard = Gossips::default();
        let mut random = LegacyRandom::new(11);
        let n = heard.transfer_from(&source, &mut random, 50);
        assert!((1..=2).contains(&n));
        // 25 - 10 per transfer for major negative, 25 - 20 for trading.
        let major = heard.get(PLAYER, GossipType::MajorNegative);
        let trading = heard.get(OTHER, GossipType::Trading);
        assert!(major == 0 || major == 15);
        assert!(trading == 0 || trading == 5);
        assert!(major + trading > 0);
        // A second hearing keeps the stronger value.
        heard.transfer_from(&source, &mut random, 50);
        assert!(heard.get(PLAYER, GossipType::MajorNegative) <= 15);
        // Gossip too weak to survive retelling is dropped.
        let mut faint = Gossips::default();
        faint.add(PLAYER, GossipType::Trading, 5);
        let mut heard = Gossips::default();
        heard.transfer_from(&faint, &mut random, 10);
        assert!(heard.is_empty());
    }
    #[test]
    fn gossip_round_trips_through_nbt() {
        let mut g = Gossips::default();
        g.add(PLAYER, GossipType::MajorNegative, 25);
        g.add(PLAYER, GossipType::Trading, 2);
        g.add(OTHER, GossipType::MinorPositive, 25);
        assert_eq!(Gossips::from_nbt(&g.to_nbt()), g);
    }
    #[test]
    fn counts_by_type() {
        let mut g = Gossips::default();
        g.add(PLAYER, GossipType::MajorNegative, 25);
        g.add(OTHER, GossipType::MajorNegative, 25);
        g.add(OTHER, GossipType::Trading, 25);
        assert_eq!(g.count_for_type(GossipType::MajorNegative, |v| v < 0.0), 2);
        assert_eq!(g.count_for_type(GossipType::Trading, |v| v < 0.0), 0);
    }
}
