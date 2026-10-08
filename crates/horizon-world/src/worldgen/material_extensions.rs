//! Eroded-badlands pillars and frozen-ocean icebergs, before chunk integration.
use super::biome_temperature::BiomeTemperature;
use crate::registry::BlockStateId;
use horizon_core::{
    noise::Noise,
    position::BlockPos,
    random::{PositionalRandom, RandomSource},
};
use std::sync::Arc;

/// Reads and writes retain provider order. The chunk adapter supplies vanilla
/// out-of-height reads and generation-height write clipping.
pub trait MaterialColumn {
    type Error;
    fn block(&mut self, y: i32) -> Result<BlockStateId, Self::Error>;
    fn set_block(&mut self, y: i32, state: BlockStateId) -> Result<(), Self::Error>;
    fn is_air(&self, state: BlockStateId) -> bool;
    fn same_block(&self, left: BlockStateId, right: BlockStateId) -> bool;
}
#[derive(Clone, Copy, Debug)]
pub struct ExtensionBlocks {
    pub default: BlockStateId,
    pub water: BlockStateId,
    pub packed_ice: BlockStateId,
    pub snow: BlockStateId,
}
#[derive(Clone, Debug)]
pub struct ExtensionNoises {
    pub badlands_surface: Arc<dyn Noise>,
    pub badlands_pillar: Arc<dyn Noise>,
    pub badlands_roof: Arc<dyn Noise>,
    pub iceberg_surface: Arc<dyn Noise>,
    pub iceberg_pillar: Arc<dyn Noise>,
    pub iceberg_roof: Arc<dyn Noise>,
}
#[derive(Debug)]
pub enum ExtensionError<E> {
    Column(E),
    IterationLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for ExtensionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Column(error) => error.fmt(f),
            Self::IterationLimit => f.write_str("material column iteration bound exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ExtensionError<E> {}
impl<E> From<E> for ExtensionError<E> {
    fn from(error: E) -> Self {
        Self::Column(error)
    }
}

/// Random is the world's root positional factory. Noises and biome temperatures
/// must be configured for the same immutable world-generation settings.
pub struct MaterialExtensions<P> {
    random: P,
    blocks: ExtensionBlocks,
    noises: ExtensionNoises,
    sea_level: i32,
}
impl<P: PositionalRandom> MaterialExtensions<P> {
    pub fn new(
        random: P,
        blocks: ExtensionBlocks,
        noises: ExtensionNoises,
        sea_level: i32,
    ) -> Self {
        Self {
            random,
            blocks,
            noises,
            sea_level,
        }
    }
    pub fn eroded_badlands<C: MaterialColumn>(
        &self,
        column: &mut C,
        x: i32,
        z: i32,
        surface_y: i32,
        min_y: i32,
    ) -> Result<(), ExtensionError<C::Error>> {
        let x = f64::from(x);
        let z = f64::from(z);
        let amplitude = java_min(
            (f64::from(self.noises.badlands_surface.get3(x, 0.0, z)) * 8.25).abs(),
            f64::from(self.noises.badlands_pillar.get3(x * 0.2, 0.0, z * 0.2) * 15.0),
        );
        if amplitude <= 0.0 {
            return Ok(());
        }
        let roof = (f64::from(self.noises.badlands_roof.get3(x * 0.75, 0.0, z * 0.75)) * 1.5).abs();
        let top = (64.0 + java_min(amplitude * amplitude * 2.5, (roof * 50.0).ceil() + 24.0))
            .floor() as i32;
        if surface_y > top {
            return Ok(());
        }
        let mut budget = 1 << 20;
        let mut y = top;
        while y >= min_y {
            tick(&mut budget)?;
            let state = column.block(y)?;
            if column.same_block(state, self.blocks.default) {
                break;
            }
            if column.same_block(state, self.blocks.water) {
                return Ok(());
            }
            y = y.wrapping_sub(1);
        }
        y = top;
        while y >= min_y {
            tick(&mut budget)?;
            let state = column.block(y)?;
            if !column.is_air(state) {
                break;
            }
            column.set_block(y, self.blocks.default)?;
            y = y.wrapping_sub(1);
        }
        Ok(())
    }
    pub fn frozen_ocean<C: MaterialColumn>(
        &self,
        column: &mut C,
        biome: &mut BiomeTemperature,
        position: [i32; 2],
        surface_y: i32,
        minimum_surface_y: i32,
    ) -> Result<(), ExtensionError<C::Error>> {
        self.frozen_ocean_with_melt(
            column,
            position,
            surface_y,
            minimum_surface_y,
            |position, sea| Ok(biome.should_melt_frozen_ocean_iceberg_slightly(position, sea)),
        )
    }
    /// Defer the biome lookup until the native noise/height checks reach it.
    pub fn frozen_ocean_with_melt<C: MaterialColumn>(
        &self,
        column: &mut C,
        position: [i32; 2],
        surface_y: i32,
        minimum_surface_y: i32,
        melt: impl FnOnce(BlockPos, i32) -> Result<bool, C::Error>,
    ) -> Result<(), ExtensionError<C::Error>> {
        let [x, z] = position;
        let dx = f64::from(x);
        let dz = f64::from(z);
        let amplitude = java_min(
            (f64::from(self.noises.iceberg_surface.get3(dx, 0.0, dz)) * 8.25).abs(),
            f64::from(self.noises.iceberg_pillar.get3(dx * 1.28, 0.0, dz * 1.28) * 15.0),
        );
        if amplitude <= 1.8 {
            return Ok(());
        }
        let roof =
            (f64::from(self.noises.iceberg_roof.get3(dx * 1.17, 0.0, dz * 1.17)) * 1.5).abs();
        let mut height = java_min(amplitude * amplitude * 1.2, (roof * 40.0).ceil() + 14.0);
        if melt(BlockPos::new(x, self.sea_level, z), self.sea_level)? {
            height -= 2.0;
        }
        if height <= 2.0 {
            return Ok(());
        }
        let bottom = f64::from(self.sea_level) - height - 7.0;
        let top = height + f64::from(self.sea_level);
        let mut random = self.random.at(BlockPos::new(x, 0, z));
        let snow_limit = 2 + random.next_int_bound(4).unwrap();
        let snow_height = self
            .sea_level
            .wrapping_add(18)
            .wrapping_add(random.next_int_bound(10).unwrap());
        let mut snow_count = 0;
        let mut y = surface_y.max((top as i32).wrapping_add(1));
        let mut budget = 1 << 20;
        while y >= minimum_surface_y {
            tick(&mut budget)?;
            let state = column.block(y)?;
            let air = column.is_air(state) && y < (top as i32) && random.next_double() > 0.01;
            let fill = if air {
                true
            } else {
                // The second getBlock is observable even when the first read
                // found air but failed its height or random test.
                let state = column.block(y)?;
                column.same_block(state, self.blocks.water)
                    && y > (bottom as i32)
                    && y < self.sea_level
                    && random.next_double() > 0.15
            };
            if fill {
                let state = if snow_count <= snow_limit && y > snow_height {
                    snow_count += 1;
                    self.blocks.snow
                } else {
                    self.blocks.packed_ice
                };
                column.set_block(y, state)?;
            }
            y = y.wrapping_sub(1);
        }
        Ok(())
    }
}
fn tick<E>(remaining: &mut usize) -> Result<(), ExtensionError<E>> {
    if *remaining == 0 {
        return Err(ExtensionError::IterationLimit);
    }
    *remaining -= 1;
    Ok(())
}
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() | b.to_bits())
    } else if a <= b {
        a
    } else {
        b
    }
}
