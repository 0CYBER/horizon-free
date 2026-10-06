//! Vanilla's two seeded random streams. No platform/default Rust RNG is used.
mod hash;
mod positional;
mod worldgen;
pub use hash::{seed_from_hash, seed_from_hash_utf16};
pub use positional::{
    LegacyPositionalRandom, PositionalRandom, XoroshiroPositionalRandom, java_string_hash,
    position_seed,
};
pub use worldgen::{WorldgenPositional, WorldgenRandom, WorldgenSource, slime_chunk_random};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBound;

impl std::fmt::Display for InvalidBound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bound must be positive")
    }
}

impl std::error::Error for InvalidBound {}

pub trait RandomSource: Sized {
    type Positional: PositionalRandom<Source = Self>;
    fn set_seed(&mut self, seed: i64);
    fn next_int(&mut self) -> i32;
    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound>;
    fn next_long(&mut self) -> i64;
    fn next_boolean(&mut self) -> bool;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
    fn fork_positional(&mut self) -> Self::Positional;
    fn fork(&mut self) -> Self
    where
        Self: Sized;

    fn consume_count(&mut self, count: i32) {
        for _ in 0..count {
            self.next_int();
        }
    }
    fn next_int_between_inclusive(&mut self, min: i32, max: i32) -> Result<i32, InvalidBound> {
        self.next_int_bound(max.wrapping_sub(min).wrapping_add(1))
            .map(|v| v.wrapping_add(min))
    }
    fn next_int_between(&mut self, origin: i32, bound: i32) -> Result<i32, InvalidBound> {
        if origin >= bound {
            return Err(InvalidBound);
        }
        self.next_int_bound(bound.wrapping_sub(origin))
            .map(|v| v.wrapping_add(origin))
    }
    fn triangle(&mut self, mean: f64, deviation: f64) -> f64 {
        mean + deviation * (self.next_double() - self.next_double())
    }
    fn triangle_float(&mut self, mean: f32, deviation: f32) -> f32 {
        mean + deviation * (self.next_float() - self.next_float())
    }
}

#[derive(Clone, Debug)]
pub struct LegacyRandom {
    state: u64,
}

impl LegacyRandom {
    const MULTIPLIER: u64 = 0x5_deec_e66d;
    const MASK: u64 = (1 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        Self {
            state: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    fn bits(&mut self, count: u32) -> i32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.state >> (48 - count)) as i32
    }
}

impl RandomSource for LegacyRandom {
    type Positional = LegacyPositionalRandom;
    fn fork_positional(&mut self) -> Self::Positional {
        LegacyPositionalRandom::new(self.next_long())
    }
    fn set_seed(&mut self, seed: i64) {
        *self = Self::new(seed);
    }

    fn next_int(&mut self) -> i32 {
        self.bits(32)
    }

    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if bound <= 0 {
            return Err(InvalidBound);
        }
        if bound & (bound - 1) == 0 {
            return Ok(((i64::from(bound) * i64::from(self.bits(31))) >> 31) as i32);
        }
        loop {
            let bits = self.bits(31);
            let value = bits % bound;
            // Java's signed overflow is part of the rejection rule.
            if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
                return Ok(value);
            }
        }
    }

    fn next_long(&mut self) -> i64 {
        let high = i64::from(self.bits(32)) << 32;
        // Java sign-extends the low int before addition; concatenation differs.
        high.wrapping_add(i64::from(self.bits(32)))
    }

    fn next_boolean(&mut self) -> bool {
        self.bits(1) != 0
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 * (1.0 / 16_777_216.0)
    }
    fn next_double(&mut self) -> f64 {
        let value = (i64::from(self.bits(26)) << 27) + i64::from(self.bits(27));
        value as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    fn fork(&mut self) -> Self {
        Self::new(self.next_long())
    }
}

#[derive(Clone, Debug)]
pub struct XoroshiroRandom {
    lo: u64,
    hi: u64,
}

impl XoroshiroRandom {
    const GOLDEN_RATIO: u64 = 0x9e37_79b9_7f4a_7c15;
    const SILVER_RATIO: u64 = 0x6a09_e667_f3bc_c909;

    pub fn new(seed: i64) -> Self {
        let lo = seed as u64 ^ Self::SILVER_RATIO;
        Self::from_state(
            mix_stafford13(lo),
            mix_stafford13(lo.wrapping_add(Self::GOLDEN_RATIO)),
        )
    }

    pub fn from_state(lo: u64, hi: u64) -> Self {
        if lo | hi == 0 {
            Self {
                lo: Self::GOLDEN_RATIO,
                hi: Self::SILVER_RATIO,
            }
        } else {
            Self { lo, hi }
        }
    }
}

impl RandomSource for XoroshiroRandom {
    type Positional = XoroshiroPositionalRandom;
    fn fork_positional(&mut self) -> Self::Positional {
        let lo = self.next_long() as u64;
        let hi = self.next_long() as u64;
        XoroshiroPositionalRandom::new(lo, hi)
    }
    fn set_seed(&mut self, seed: i64) {
        *self = Self::new(seed);
    }
    fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if bound <= 0 {
            return Err(InvalidBound);
        }
        let bound = bound as u32;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let product = u64::from(self.next_int() as u32) * u64::from(bound);
            if product as u32 >= threshold {
                return Ok((product >> 32) as i32);
            }
        }
    }

    fn next_long(&mut self) -> i64 {
        let lo = self.lo;
        let hi = self.hi ^ lo;
        let result = lo.wrapping_add(self.hi).rotate_left(17).wrapping_add(lo);
        self.lo = lo.rotate_left(49) ^ hi ^ (hi << 21);
        self.hi = hi.rotate_left(28);
        result as i64
    }

    fn next_boolean(&mut self) -> bool {
        self.next_long() & 1 != 0
    }
    fn next_float(&mut self) -> f32 {
        ((self.next_long() as u64) >> 40) as f32 * (1.0 / 16_777_216.0)
    }
    fn next_double(&mut self) -> f64 {
        ((self.next_long() as u64) >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
    fn fork(&mut self) -> Self {
        let lo = self.next_long() as u64;
        let hi = self.next_long() as u64;
        Self::from_state(lo, hi)
    }
}

pub const fn mix_stafford13(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_jdk_sequence_and_sign_extended_low_word() {
        let mut rng = LegacyRandom::new(0);
        assert_eq!(rng.next_int(), -1_155_484_576);
        assert_eq!(rng.next_int(), -723_955_400);
        assert_eq!(LegacyRandom::new(0).next_long(), -4_962_768_465_676_381_896);
    }

    fn invalid_bounds_do_not_consume_state<R: RandomSource + Clone>(mut rng: R) {
        let mut reference = rng.clone();
        for bound in [0, -1, i32::MIN] {
            assert_eq!(rng.next_int_bound(bound), Err(InvalidBound));
        }
        rng.consume_count(-1);
        assert_eq!(rng.next_long(), reference.next_long());
    }

    #[test]
    fn invalid_inputs_preserve_both_streams() {
        invalid_bounds_do_not_consume_state(LegacyRandom::new(0));
        invalid_bounds_do_not_consume_state(XoroshiroRandom::new(0));
    }

    #[test]
    fn zero_state_fallback_and_reseed() {
        let mut a = XoroshiroRandom::from_state(0, 0);
        let mut b = XoroshiroRandom::from_state(
            XoroshiroRandom::GOLDEN_RATIO,
            XoroshiroRandom::SILVER_RATIO,
        );
        for _ in 0..100 {
            assert_eq!(a.next_long(), b.next_long());
        }
        a.set_seed(-1);
        assert_eq!(a.next_long(), XoroshiroRandom::new(-1).next_long());
    }
}
