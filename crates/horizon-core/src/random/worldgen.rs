//! Legacy bit-sampling adapter used by vanilla's world-generation random stream.
use super::{InvalidBound, LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom};
use crate::position::BlockPos;

mod sealed {
    pub trait Sealed {}
}
pub trait WorldgenSource: RandomSource + sealed::Sealed {
    #[doc(hidden)]
    fn worldgen_bits(&mut self, count: i32) -> i32;
}
impl sealed::Sealed for LegacyRandom {}
impl WorldgenSource for LegacyRandom {
    fn worldgen_bits(&mut self, count: i32) -> i32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        self.state.wrapping_shr(48_i32.wrapping_sub(count) as u32) as i32
    }
}
impl sealed::Sealed for XoroshiroRandom {}
impl WorldgenSource for XoroshiroRandom {
    fn worldgen_bits(&mut self, count: i32) -> i32 {
        (self.next_long() as u64).wrapping_shr(64_i32.wrapping_sub(count) as u32) as i32
    }
}

#[derive(Clone, Debug)]
pub struct WorldgenRandom<R: WorldgenSource> {
    source: R,
    count: i32,
    wrapped: bool,
    /// The adapter's own Gaussian cache; native setSeed does not reset it.
    gaussian: Option<f64>,
}
impl<R: WorldgenSource> WorldgenRandom<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            count: 0,
            wrapped: true,
            gaussian: None,
        }
    }
    fn plain(source: R) -> Self {
        Self {
            source,
            count: 0,
            wrapped: false,
            gaussian: None,
        }
    }
    pub fn count(&self) -> i32 {
        self.count
    }
    pub fn next_bits(&mut self, bits: i32) -> i32 {
        if self.wrapped {
            self.count = self.count.wrapping_add(1);
        }
        self.source.worldgen_bits(bits)
    }
    pub fn set_decoration_seed(&mut self, seed: i64, x: i32, z: i32) -> i64 {
        self.set_seed(seed);
        let a = self.next_long() | 1;
        let b = self.next_long() | 1;
        let mixed = (x as i64)
            .wrapping_mul(a)
            .wrapping_add((z as i64).wrapping_mul(b))
            ^ seed;
        self.set_seed(mixed);
        mixed
    }
    pub fn set_feature_seed(&mut self, seed: i64, index: i32, step: i32) {
        self.set_seed(
            seed.wrapping_add(index as i64)
                .wrapping_add(step.wrapping_mul(10000) as i64),
        );
    }
    pub fn set_large_feature_seed(&mut self, seed: i64, x: i32, z: i32) {
        self.set_seed(seed);
        let a = self.next_long();
        let b = self.next_long();
        self.set_seed((x as i64).wrapping_mul(a) ^ (z as i64).wrapping_mul(b) ^ seed);
    }
    pub fn set_large_feature_with_salt(&mut self, seed: i64, x: i32, z: i32, salt: i32) {
        self.set_seed(
            (x as i64)
                .wrapping_mul(341873128712)
                .wrapping_add((z as i64).wrapping_mul(132897987541))
                .wrapping_add(seed)
                .wrapping_add(salt as i64),
        );
    }
}
impl<R: WorldgenSource> sealed::Sealed for WorldgenRandom<R> {}
impl<R: WorldgenSource> WorldgenSource for WorldgenRandom<R> {
    fn worldgen_bits(&mut self, count: i32) -> i32 {
        self.next_bits(count)
    }
}
impl<R: WorldgenSource> RandomSource for WorldgenRandom<R> {
    type Positional = WorldgenPositional<R::Positional>;
    fn set_seed(&mut self, seed: i64) {
        self.source.set_seed(seed);
    }
    fn next_int(&mut self) -> i32 {
        if self.wrapped {
            self.next_bits(32)
        } else {
            self.source.next_int()
        }
    }
    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if !self.wrapped {
            return self.source.next_int_bound(bound);
        }
        if bound <= 0 {
            return Err(InvalidBound);
        }
        if bound & (bound - 1) == 0 {
            return Ok(((bound as i64 * self.next_bits(31) as i64) >> 31) as i32);
        }
        loop {
            let bits = self.next_bits(31);
            let value = bits % bound;
            if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
                return Ok(value);
            }
        }
    }
    fn next_long(&mut self) -> i64 {
        if !self.wrapped {
            return self.source.next_long();
        }
        let high = (self.next_bits(32) as i64) << 32;
        high.wrapping_add(self.next_bits(32) as i64)
    }
    fn next_boolean(&mut self) -> bool {
        if self.wrapped {
            self.next_bits(1) != 0
        } else {
            self.source.next_boolean()
        }
    }
    fn next_float(&mut self) -> f32 {
        if self.wrapped {
            self.next_bits(24) as f32 * (1.0 / 16777216.0)
        } else {
            self.source.next_float()
        }
    }
    fn next_double(&mut self) -> f64 {
        if !self.wrapped {
            return self.source.next_double();
        }
        let bits = ((self.next_bits(26) as i64) << 27) + self.next_bits(27) as i64;
        bits as f64 * (1.0 / 9007199254740992.0)
    }
    fn next_gaussian(&mut self) -> f64 {
        if !self.wrapped {
            return self.source.next_gaussian();
        }
        let mut cache = self.gaussian.take();
        let value = super::marsaglia_polar(&mut cache, || self.next_double());
        self.gaussian = cache;
        value
    }
    /// Native forks return the underlying source, not another bit adapter.
    /// The returned value keeps that behavior through a transparent variant.
    fn fork(&mut self) -> Self {
        Self::plain(self.source.fork())
    }
    fn fork_positional(&mut self) -> Self::Positional {
        WorldgenPositional(self.source.fork_positional())
    }
}
#[derive(Clone, Debug)]
pub struct WorldgenPositional<P: PositionalRandom>(P);
impl<P: PositionalRandom> PositionalRandom for WorldgenPositional<P>
where
    P::Source: WorldgenSource,
{
    type Source = WorldgenRandom<P::Source>;
    fn at(&self, position: BlockPos) -> Self::Source {
        WorldgenRandom::plain(self.0.at(position))
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        WorldgenRandom::plain(self.0.from_seed(seed))
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        WorldgenRandom::plain(self.0.from_hash_of(name))
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        WorldgenRandom::plain(self.0.from_hash_of_utf16(name))
    }
}
pub fn slime_chunk_random(x: i32, z: i32, seed: i64, salt: i64) -> LegacyRandom {
    let mixed = seed
        .wrapping_add(x.wrapping_mul(x).wrapping_mul(4987142) as i64)
        .wrapping_add(x.wrapping_mul(5947611) as i64)
        .wrapping_add((z.wrapping_mul(z) as i64).wrapping_mul(4392871))
        .wrapping_add(z.wrapping_mul(389711) as i64)
        ^ salt;
    LegacyRandom::new(mixed)
}
