//! Placement modifiers that depend on random state and generation bounds.
//! Block-query modifiers and actual configured features remain separate work.
mod chain;
mod layer;
use super::block_predicate::{BlockPredicate, PredicateBlockResolver};
use super::{
    DecodeError, Object, anchor::WorldGenerationContext, biome_temperature::biome_info_noise,
    density::identifier, double, height_provider::HeightProvider, int_range, integer, number_text,
    optional, required, value_provider::IntProvider,
};
use crate::heightmap::HeightmapType;
pub use chain::{FeaturePlacementError, FeaturePlacer};
use horizon_core::random::{InvalidBound, RandomSource};
use serde_json::value::RawValue;
mod context;
pub use context::{LayerBlock, PlacementContext, PlacementQueryError};

#[derive(Clone, Debug)]
pub enum PlacementModifier {
    Biome,
    BlockPredicate(BlockPredicate),
    EnvironmentScan {
        upwards: bool,
        target: BlockPredicate,
        allowed: BlockPredicate,
        max_steps: i32,
    },
    Heightmap(HeightmapType),
    SurfaceRelative {
        heightmap: HeightmapType,
        min: i32,
        max: i32,
    },
    SurfaceWaterDepth(i32),
    Count(IntProvider),
    CountOnEveryLayer(IntProvider),
    NoiseBasedCount {
        ratio: i32,
        factor: f64,
        offset: f64,
    },
    NoiseThresholdCount {
        level: f64,
        below: i32,
        above: i32,
    },
    InSquare,
    Cuboid {
        xz_size: IntProvider,
        y_size: IntProvider,
        include_edges: bool,
        include_interior: bool,
    },
    Offset([IntProvider; 3]),
    HeightRange(HeightProvider),
    Rarity(i32),
    RandomChance(f32),
    Fixed(Vec<[i32; 3]>),
    RandomlySelected(Vec<Self>),
}
#[derive(Debug)]
pub enum PlacementError<E> {
    Random(InvalidBound),
    World(PlacementQueryError),
    Output(E),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for PlacementError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
            Self::Output(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("placement work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for PlacementError<E> {}
impl<E> From<InvalidBound> for PlacementError<E> {
    fn from(value: InvalidBound) -> Self {
        Self::Random(value)
    }
}
impl<E> From<PlacementQueryError> for PlacementError<E> {
    fn from(value: PlacementQueryError) -> Self {
        Self::World(value)
    }
}
impl PlacementModifier {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        Self::decode_with_blocks(json, depth, remaining, None)
    }
    pub fn from_json_with_blocks(
        json: &str,
        blocks: &impl PredicateBlockResolver,
    ) -> Result<Self, DecodeError> {
        Self::decode_with_blocks(json, 0, &mut 65536, Some(blocks))
    }
    pub(crate) fn decode_with_blocks(
        json: &str,
        depth: usize,
        remaining: &mut usize,
        blocks: Option<&dyn PredicateBlockResolver>,
    ) -> Result<Self, DecodeError> {
        if depth > 256 || *remaining == 0 {
            return Err(DecodeError::Field("placement depth or node count"));
        }
        *remaining -= 1;
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        let provider = |name, min, max| {
            let value = IntProvider::from_json(required(&object, name)?.get())?;
            let bounds = value.bounds();
            if bounds[0] < min || bounds[1] > max {
                return Err(DecodeError::Field("placement provider bounds"));
            }
            Ok(value)
        };
        let heightmap = || {
            let name: String = serde_json::from_str(required(&object, "heightmap")?.get())?;
            HeightmapType::ALL
                .into_iter()
                .find(|kind| kind.name() == name)
                .ok_or(DecodeError::Field("heightmap"))
        };
        match kind.as_str() {
            "minecraft:biome" => Ok(Self::Biome),
            "minecraft:block_predicate_filter" => {
                Ok(Self::BlockPredicate(BlockPredicate::decode_with_blocks(
                    required(&object, "predicate")?.get(),
                    depth + 1,
                    remaining,
                    blocks,
                )?))
            }
            "minecraft:environment_scan" => {
                let direction: String =
                    serde_json::from_str(required(&object, "direction_of_search")?.get())?;
                let upwards = match direction.as_str() {
                    "up" => true,
                    "down" => false,
                    _ => return Err(DecodeError::Field("scan direction")),
                };
                Ok(Self::EnvironmentScan {
                    upwards,
                    target: BlockPredicate::decode_with_blocks(
                        required(&object, "target_condition")?.get(),
                        depth + 1,
                        remaining,
                        blocks,
                    )?,
                    allowed: optional(&object, "allowed_search_condition")
                        .map(|v| {
                            BlockPredicate::decode_with_blocks(
                                v.get(),
                                depth + 1,
                                remaining,
                                blocks,
                            )
                        })
                        .transpose()?
                        .unwrap_or(BlockPredicate::True),
                    max_steps: int_range(required(&object, "max_steps")?, "scan steps", 1, 32)?,
                })
            }
            "minecraft:heightmap" => Ok(Self::Heightmap(heightmap()?)),
            "minecraft:surface_relative_threshold_filter" => Ok(Self::SurfaceRelative {
                heightmap: heightmap()?,
                min: optional(&object, "min_inclusive")
                    .map(integer)
                    .transpose()?
                    .unwrap_or(i32::MIN),
                max: optional(&object, "max_inclusive")
                    .map(integer)
                    .transpose()?
                    .unwrap_or(i32::MAX),
            }),
            "minecraft:surface_water_depth_filter" => Ok(Self::SurfaceWaterDepth(integer(
                required(&object, "max_water_depth")?,
            )?)),
            "minecraft:noise_based_count" => Ok(Self::NoiseBasedCount {
                ratio: integer(required(&object, "noise_to_count_ratio")?)?,
                factor: double(required(&object, "noise_factor")?)?,
                offset: optional(&object, "noise_offset")
                    .map(double)
                    .transpose()?
                    .unwrap_or(0.),
            }),
            "minecraft:noise_threshold_count" => Ok(Self::NoiseThresholdCount {
                level: double(required(&object, "noise_level")?)?,
                below: integer(required(&object, "below_noise")?)?,
                above: integer(required(&object, "above_noise")?)?,
            }),
            "minecraft:count_on_every_layer" => {
                Ok(Self::CountOnEveryLayer(provider("count", 0, 256)?))
            }
            "minecraft:count" => Ok(Self::Count(provider("count", 0, 4096)?)),
            "minecraft:in_square" => Ok(Self::InSquare),
            "minecraft:cuboid" => Ok(Self::Cuboid {
                xz_size: provider("xz_size", 1, 16)?,
                y_size: provider("y_size", 1, 16)?,
                include_edges: optional(&object, "include_edges")
                    .map(|raw| serde_json::from_str(raw.get()))
                    .transpose()?
                    .unwrap_or(true),
                include_interior: optional(&object, "include_interior")
                    .map(|raw| serde_json::from_str(raw.get()))
                    .transpose()?
                    .unwrap_or(true),
            }),
            "minecraft:offset" => Ok(Self::Offset([
                provider("x", -16, 16)?,
                provider("y", -16, 16)?,
                provider("z", -16, 16)?,
            ])),
            "minecraft:height_range" => Ok(Self::HeightRange(HeightProvider::from_json(
                required(&object, "height")?.get(),
            )?)),
            "minecraft:rarity_filter" => Ok(Self::Rarity(int_range(
                required(&object, "chance")?,
                "chance",
                1,
                i32::MAX,
            )?)),
            "minecraft:random_chance" => {
                let chance: f32 = number_text(required(&object, "chance")?)?
                    .parse()
                    .map_err(|_| DecodeError::Field("chance"))?;
                if !(0. ..=1.).contains(&chance) || chance.is_sign_negative() {
                    return Err(DecodeError::Field("chance"));
                }
                Ok(Self::RandomChance(chance))
            }
            "minecraft:fixed_placement" => {
                let values: Vec<Box<RawValue>> =
                    serde_json::from_str(required(&object, "positions")?.get())?;
                if values.len() > 8192 {
                    return Err(DecodeError::Field("fixed positions"));
                }
                let mut positions = Vec::with_capacity(values.len());
                for raw in values {
                    let coords: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
                    if coords.len() != 3 {
                        return Err(DecodeError::Field("block position"));
                    }
                    positions.push([
                        integer(&coords[0])?,
                        integer(&coords[1])?,
                        integer(&coords[2])?,
                    ]);
                }
                Ok(Self::Fixed(positions))
            }
            "minecraft:randomly_selected" => {
                let values: Vec<Box<RawValue>> =
                    serde_json::from_str(required(&object, "placements")?.get())?;
                if values.is_empty() || values.len() > 8192 {
                    return Err(DecodeError::Field("selected placements"));
                }
                Ok(Self::RandomlySelected(
                    values
                        .iter()
                        .map(|v| Self::decode_with_blocks(v.get(), depth + 1, remaining, blocks))
                        .collect::<Result<_, _>>()?,
                ))
            }
            _ => Err(DecodeError::Field("placement modifier type")),
        }
    }
    /// Return the count for repeating modifiers without allocating or emitting
    /// positions. Sampling still consumes the count provider's random draws.
    pub fn repetitions(
        &self,
        random: &mut impl RandomSource,
        position: [i32; 3],
    ) -> Result<Option<i32>, InvalidBound> {
        let [x, _, z] = position;
        Ok(match self {
            Self::Count(provider) => Some(provider.sample(random)?),
            Self::NoiseBasedCount {
                ratio,
                factor,
                offset,
            } => {
                let noise = f64::from(biome_info_noise(
                    f64::from(x) / factor,
                    f64::from(z) / factor,
                ));
                Some(((noise + offset) * f64::from(*ratio)).ceil() as i32)
            }
            Self::NoiseThresholdCount {
                level,
                below,
                above,
            } => {
                let noise = f64::from(biome_info_noise(f64::from(x) / 200., f64::from(z) / 200.));
                Some(if noise < *level { *below } else { *above })
            }
            _ => None,
        })
    }
    /// Emit positions in native callback order. The callback shares the random
    /// stream; failures retain prior emissions and consumed draws.
    pub fn modify<R: RandomSource, E>(
        &self,
        mut context: WorldGenerationContext,
        random: &mut R,
        position: [i32; 3],
        emit: impl FnMut(&mut R, [i32; 3]) -> Result<(), E>,
    ) -> Result<(), PlacementError<E>> {
        self.modify_in(&mut context, random, position, emit)
    }
    pub fn modify_in<R: RandomSource, E>(
        &self,
        context: &mut impl PlacementContext,
        random: &mut R,
        position: [i32; 3],
        mut emit: impl FnMut(&mut R, [i32; 3]) -> Result<(), E>,
    ) -> Result<(), PlacementError<E>> {
        self.modify_inner(context, random, position, &mut emit, &mut 65536)
    }
    fn modify_inner<R: RandomSource, E>(
        &self,
        context: &mut impl PlacementContext,
        random: &mut R,
        position: [i32; 3],
        emit: &mut impl FnMut(&mut R, [i32; 3]) -> Result<(), E>,
        remaining: &mut usize,
    ) -> Result<(), PlacementError<E>> {
        if *remaining == 0 {
            return Err(PlacementError::WorkLimit);
        }
        *remaining -= 1;
        let [x, y, z] = position;
        match self {
            Self::Biome => {
                let feature = context
                    .top_feature()
                    .ok_or(PlacementQueryError::Unavailable)?;
                if context.biome_allows_feature(position, feature)? {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::BlockPredicate(predicate) => {
                if predicate.test_inner(context, position, 0, remaining)? {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::EnvironmentScan {
                upwards,
                target,
                allowed,
                max_steps,
            } => {
                let mut position = position;
                if !allowed.test_inner(context, position, 0, remaining)? {
                    return Ok(());
                }
                for _ in 0..*max_steps {
                    if target.test_inner(context, position, 0, remaining)? {
                        return emit(random, position).map_err(PlacementError::Output);
                    }
                    position[1] = position[1].wrapping_add(if *upwards { 1 } else { -1 });
                    if context.outside_build_height(position[1])? {
                        return Ok(());
                    }
                    if !allowed.test_inner(context, position, 0, remaining)? {
                        break;
                    }
                }
                if target.test_inner(context, position, 0, remaining)? {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::CountOnEveryLayer(count) => {
                layer::modify(count, context, random, position, emit, remaining)?
            }
            Self::Heightmap(kind) => {
                let height = context.height(*kind, x, z)?;
                if height > context.min_y()? {
                    emit(random, [x, height, z]).map_err(PlacementError::Output)?;
                }
            }
            Self::SurfaceRelative {
                heightmap,
                min,
                max,
            } => {
                let height = i64::from(context.height(*heightmap, x, z)?);
                if height + i64::from(*min) <= i64::from(y)
                    && i64::from(y) <= height + i64::from(*max)
                {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::SurfaceWaterDepth(max) => {
                let floor = context.height(HeightmapType::OceanFloor, x, z)?;
                let surface = context.height(HeightmapType::WorldSurface, x, z)?;
                if surface.wrapping_sub(floor) <= *max {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::Count(_) | Self::NoiseBasedCount { .. } | Self::NoiseThresholdCount { .. } => {
                let count = self
                    .repetitions(random, position)?
                    .expect("repeating modifier");
                if count > 4096 {
                    return Err(PlacementError::WorkLimit);
                }
                for _ in 0..count {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::Cuboid {
                xz_size,
                y_size,
                include_edges,
                include_interior,
            } => {
                let ny = y_size.sample(random)?;
                let nx = xz_size.sample(random)?;
                let nz = xz_size.sample(random)?;
                if nx < 0 || ny < 0 || nz < 0 {
                    return Ok(());
                }
                for dx in 0..=nx {
                    for dy in 0..=ny {
                        for dz in 0..=nz {
                            if *remaining == 0 {
                                return Err(PlacementError::WorkLimit);
                            }
                            *remaining -= 1;
                            let bx = dx == 0 || dx == nx;
                            let by = dy == 0 || dy == ny;
                            let bz = dz == 0 || dz == nz;
                            if !include_edges && (u8::from(bx) + u8::from(by) + u8::from(bz)) >= 2 {
                                continue;
                            }
                            if !(*include_interior || bx || by || bz) {
                                continue;
                            }
                            emit(
                                random,
                                [x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz)],
                            )
                            .map_err(PlacementError::Output)?;
                        }
                    }
                }
            }
            Self::InSquare => {
                let x = x.wrapping_add(random.next_int_bound(16)?);
                let z = z.wrapping_add(random.next_int_bound(16)?);
                emit(random, [x, y, z]).map_err(PlacementError::Output)?;
            }
            Self::Offset(offsets) => {
                let x = x.wrapping_add(offsets[0].sample(random)?);
                let y = y.wrapping_add(offsets[1].sample(random)?);
                let z = z.wrapping_add(offsets[2].sample(random)?);
                emit(random, [x, y, z]).map_err(PlacementError::Output)?;
            }
            Self::HeightRange(height) => {
                let y = height.sample(random, context.generation())?;
                emit(random, [x, y, z]).map_err(PlacementError::Output)?;
            }
            Self::Rarity(chance) => {
                if random.next_float() < 1. / *chance as f32 {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::RandomChance(chance) => {
                if random.next_float() < *chance {
                    emit(random, position).map_err(PlacementError::Output)?;
                }
            }
            Self::Fixed(positions) => {
                if positions.len() > 8192 {
                    return Err(PlacementError::WorkLimit);
                }
                for p in positions {
                    if p[0] >> 4 == x >> 4 && p[2] >> 4 == z >> 4 {
                        emit(random, *p).map_err(PlacementError::Output)?;
                    }
                }
            }
            Self::RandomlySelected(placements) => {
                if placements.len() > 8192 {
                    return Err(PlacementError::WorkLimit);
                }
                let index = random.next_int_bound(placements.len() as i32)? as usize;
                placements[index].modify_inner(context, random, position, emit, remaining)?;
            }
        }
        Ok(())
    }
}
