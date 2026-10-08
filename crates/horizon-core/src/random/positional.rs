use super::{LegacyRandom, RandomSource, XoroshiroRandom, seed_from_hash, seed_from_hash_utf16};
use crate::position::BlockPos;

#[allow(clippy::wrong_self_convention)] // These are seeded factories, matching Java's fromSeed/fromHashOf.
pub trait PositionalRandom {
    type Source: RandomSource;
    fn at(&self, position: BlockPos) -> Self::Source;
    fn from_seed(&self, seed: i64) -> Self::Source;
    fn from_hash_of(&self, name: &str) -> Self::Source;
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyPositionalRandom {
    seed: i64,
}
impl LegacyPositionalRandom {
    pub const fn new(seed: i64) -> Self {
        Self { seed }
    }
}
impl PositionalRandom for LegacyPositionalRandom {
    type Source = LegacyRandom;
    fn at(&self, position: BlockPos) -> Self::Source {
        LegacyRandom::new(position_seed(position) ^ self.seed)
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        LegacyRandom::new(seed)
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        LegacyRandom::new(java_string_hash(name.encode_utf16()) as i64 ^ self.seed)
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        LegacyRandom::new(java_string_hash(name.iter().copied()) as i64 ^ self.seed)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XoroshiroPositionalRandom {
    lo: u64,
    hi: u64,
}
impl XoroshiroPositionalRandom {
    pub const fn new(lo: u64, hi: u64) -> Self {
        Self { lo, hi }
    }
}
impl PositionalRandom for XoroshiroPositionalRandom {
    type Source = XoroshiroRandom;
    fn at(&self, position: BlockPos) -> Self::Source {
        XoroshiroRandom::from_state(position_seed(position) as u64 ^ self.lo, self.hi)
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        XoroshiroRandom::from_state(seed as u64 ^ self.lo, seed as u64 ^ self.hi)
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        let [lo, hi] = seed_from_hash(name);
        XoroshiroRandom::from_state(lo ^ self.lo, hi ^ self.hi)
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        let [lo, hi] = seed_from_hash_utf16(name);
        XoroshiroRandom::from_state(lo ^ self.lo, hi ^ self.hi)
    }
}
pub fn java_string_hash(name: impl IntoIterator<Item = u16>) -> i32 {
    name.into_iter().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(unit as i32)
    })
}
/// Mth.getSeed's signed Java arithmetic, including the initial int product.
pub const fn position_seed(position: BlockPos) -> i64 {
    let mixed = position.x.wrapping_mul(3_129_871) as i64
        ^ (position.z as i64).wrapping_mul(116_129_781)
        ^ position.y as i64;
    mixed
        .wrapping_mul(mixed)
        .wrapping_mul(42_317_861)
        .wrapping_add(mixed.wrapping_mul(11))
        >> 16
}
