//! Ore placement with ordered replacement rules and air-exposure checks.
use super::{
    DecodeError, Object,
    block_feature::{BlockReplacements, FeatureBlockBindings, decode_targets},
    density::identifier,
    feature::{FeatureError, FeatureWorld},
    feature_graph::FeatureExecutionError,
    int_range, number_text,
    placement::PlacementContext,
    required,
};
use crate::heightmap::HeightmapType;
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, random::RandomSource};

#[derive(Clone, Debug)]
pub struct ScatteredOreFeature {
    targets: BlockReplacements,
    size: i32,
    discard: f32,
    air: [BlockStateId; 3],
}
impl ScatteredOreFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:scattered_ore" {
            return Err(DecodeError::Field("scattered ore type"));
        }
        let targets = decode_targets(&object, bindings, depth, remaining)?;
        let size = int_range(required(&object, "size")?, "ore size", 0, 64)?;
        let discard: f32 = number_text(required(&object, "discard_chance_on_air_exposure")?)?
            .parse()
            .map_err(|_| DecodeError::Field("ore air exposure"))?;
        if !(0.0..=1.0).contains(&discard) || discard.is_sign_negative() {
            return Err(DecodeError::Field("ore air exposure"));
        }
        Ok(Self {
            targets,
            size,
            discard,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, FeatureError<W::Error>> {
        let count = random
            .next_int_bound(self.size + 1)
            .map_err(FeatureError::Random)?;
        for attempt in 0..count {
            let spread = attempt.min(7) as f32;
            let position = origin.map(|component| {
                let offset = (random.next_float() - random.next_float()) * spread;
                // Widen before adding 0.5 so a float just below a tie cannot
                // round up during addition. Values are bounded by +/-7.
                let offset = (f64::from(offset) + 0.5).floor() as i32;
                component.wrapping_add(offset)
            });
            let current = world.block(position).map_err(FeatureError::World)?;
            for (rule, state) in &self.targets {
                if !rule.test(current, position, random) {
                    continue;
                }
                let skip_air = self.discard <= 0.0
                    || (self.discard < 1.0 && random.next_float() >= self.discard);
                if !skip_air {
                    let mut exposed = false;
                    for direction in Direction::ALL {
                        let delta = direction.normal();
                        let neighbor = std::array::from_fn(|i| position[i].wrapping_add(delta[i]));
                        if self
                            .air
                            .contains(&world.block(neighbor).map_err(FeatureError::World)?)
                        {
                            exposed = true;
                            break;
                        }
                    }
                    if exposed {
                        continue;
                    }
                }
                world
                    .set_block(position, *state, 2)
                    .map_err(FeatureError::World)?;
                break;
            }
        }
        Ok(true)
    }
}

/// Vanilla `minecraft:ore`: an interpolated chain of spheres written straight
/// into chunk sections (BulkSectionAccess), bypassing block update effects.
#[derive(Clone, Debug)]
pub struct OreFeature {
    targets: BlockReplacements,
    size: i32,
    discard: f32,
    air: [BlockStateId; 3],
}
impl OreFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:ore" {
            return Err(DecodeError::Field("ore type"));
        }
        let targets = decode_targets(&object, bindings, depth, remaining)?;
        let size = int_range(required(&object, "size")?, "ore size", 0, 64)?;
        let discard: f32 = number_text(required(&object, "discard_chance_on_air_exposure")?)?
            .parse()
            .map_err(|_| DecodeError::Field("ore air exposure"))?;
        if !(0.0..=1.0).contains(&discard) || discard.is_sign_negative() {
            return Err(DecodeError::Field("ore air exposure"));
        }
        Ok(Self {
            targets,
            size,
            discard,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        let dir = random.next_float() * std::f32::consts::PI;
        let spread_xy = self.size as f32 / 8.0;
        let max_radius = ((self.size as f32 / 16.0 * 2.0 + 1.0) / 2.0).ceil() as i32;
        let (ox, oz) = (f64::from(origin[0]), f64::from(origin[2]));
        let (sin, cos) = (
            horizon_core::jmath::sin(f64::from(dir)),
            horizon_core::jmath::cos(f64::from(dir)),
        );
        let spread = f64::from(spread_xy);
        let x0 = ox + sin * spread;
        let x1 = ox - sin * spread;
        let z0 = oz + cos * spread;
        let z1 = oz - cos * spread;
        let y0 = f64::from(
            origin[1]
                .wrapping_add(random.next_int_bound(3)?)
                .wrapping_sub(2),
        );
        let y1 = f64::from(
            origin[1]
                .wrapping_add(random.next_int_bound(3)?)
                .wrapping_sub(2),
        );
        let spread_ceil = spread.ceil() as i32;
        let x_start = origin[0].wrapping_sub(spread_ceil).wrapping_sub(max_radius);
        let y_start = origin[1].wrapping_sub(2).wrapping_sub(max_radius);
        let z_start = origin[2].wrapping_sub(spread_ceil).wrapping_sub(max_radius);
        let size_xz = 2 * (spread_ceil + max_radius);
        let size_y = 2 * (2 + max_radius);
        if let Some(found) = world
            .any_height_at_least(
                HeightmapType::OceanFloorWg,
                [x_start, z_start],
                size_xz,
                y_start,
            )
            .map_err(FeatureExecutionError::Query)?
        {
            return match found {
                true => self.do_place(
                    world,
                    random,
                    [x0, x1, z0, z1, y0, y1],
                    [x_start, y_start, z_start],
                    size_xz,
                    size_y,
                ),
                false => Ok(false),
            };
        }
        for x in 0..=size_xz {
            for z in 0..=size_xz {
                let (px, pz) = (x_start.wrapping_add(x), z_start.wrapping_add(z));
                let floor = world
                    .height(HeightmapType::OceanFloorWg, px, pz)
                    .map_err(FeatureExecutionError::Query)?;
                if y_start <= floor {
                    return self.do_place(
                        world,
                        random,
                        [x0, x1, z0, z1, y0, y1],
                        [x_start, y_start, z_start],
                        size_xz,
                        size_y,
                    );
                }
            }
        }
        Ok(false)
    }
    fn do_place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        [x0, x1, z0, z1, y0, y1]: [f64; 6],
        [x_start, y_start, z_start]: [i32; 3],
        size_xz: i32,
        size_y: i32,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        let size = self.size as usize;
        let mut data = vec![[0f64; 4]; size];
        for (i, entry) in data.iter_mut().enumerate() {
            let step = i as f32 / self.size as f32;
            let step64 = f64::from(step);
            let xx = x0 + step64 * (x1 - x0);
            let yy = y0 + step64 * (y1 - y0);
            let zz = z0 + step64 * (z1 - z0);
            let ss = random.next_double() * f64::from(self.size) / 16.0;
            let sine = horizon_core::geometry::trig::sin(f64::from(std::f32::consts::PI * step));
            let r = (f64::from(sine + 1.0) * ss + 1.0) / 2.0;
            *entry = [xx, yy, zz, r];
        }
        for i1 in 0..size.saturating_sub(1) {
            if data[i1][3] <= 0.0 {
                continue;
            }
            for i2 in i1 + 1..size {
                if data[i2][3] <= 0.0 {
                    continue;
                }
                let dx = data[i1][0] - data[i2][0];
                let dy = data[i1][1] - data[i2][1];
                let dz = data[i1][2] - data[i2][2];
                let dr = data[i1][3] - data[i2][3];
                if dr * dr > dx * dx + dy * dy + dz * dz {
                    if dr > 0.0 {
                        data[i2][3] = -1.0;
                    } else {
                        data[i1][3] = -1.0;
                    }
                }
            }
        }
        let tested_len = (size_xz as usize) * (size_y as usize) * (size_xz as usize);
        let mut tested = vec![0u64; tested_len.div_ceil(64)];
        let mut placed = 0;
        for [xx, yy, zz, r] in data {
            if r < 0.0 {
                continue;
            }
            let x_min = ((xx - r).floor() as i32).max(x_start);
            let y_min = ((yy - r).floor() as i32).max(y_start);
            let z_min = ((zz - r).floor() as i32).max(z_start);
            let x_max = ((xx + r).floor() as i32).max(x_min);
            let y_max = ((yy + r).floor() as i32).max(y_min);
            let z_max = ((zz + r).floor() as i32).max(z_min);
            for x in x_min..=x_max {
                let xd = (f64::from(x) + 0.5 - xx) / r;
                if xd * xd >= 1.0 {
                    continue;
                }
                for y in y_min..=y_max {
                    let yd = (f64::from(y) + 0.5 - yy) / r;
                    if xd * xd + yd * yd >= 1.0 {
                        continue;
                    }
                    for z in z_min..=z_max {
                        let zd = (f64::from(z) + 0.5 - zz) / r;
                        if xd * xd + yd * yd + zd * zd >= 1.0
                            || world
                                .outside_build_height(y)
                                .map_err(FeatureExecutionError::Query)?
                        {
                            continue;
                        }
                        let index = i64::from(x.wrapping_sub(x_start))
                            + i64::from(y.wrapping_sub(y_start)) * i64::from(size_xz)
                            + i64::from(z.wrapping_sub(z_start))
                                * i64::from(size_xz)
                                * i64::from(size_y);
                        // The sphere bounds keep indices inside the native BitSet.
                        let Some(index) = usize::try_from(index).ok().filter(|&i| i < tested_len)
                        else {
                            return Err(FeatureExecutionError::WorkLimit);
                        };
                        let (word, bit) = (&mut tested[index / 64], 1u64 << (index % 64));
                        if *word & bit != 0 {
                            continue;
                        }
                        *word |= bit;
                        let p = [x, y, z];
                        if !world
                            .ensure_can_write(p)
                            .map_err(FeatureExecutionError::Query)?
                        {
                            continue;
                        }
                        let Some(state) = world
                            .section_block(p)
                            .map_err(FeatureExecutionError::Query)?
                        else {
                            continue;
                        };
                        for (rule, target) in &self.targets {
                            if !rule.test(state, p, random) {
                                continue;
                            }
                            if !skip_air_check(random, self.discard) {
                                let mut exposed = false;
                                for direction in Direction::ALL {
                                    let n = direction.normal();
                                    let q = std::array::from_fn(|i| p[i].wrapping_add(n[i]));
                                    let neighbor = world
                                        .section_block(q)
                                        .map_err(FeatureExecutionError::Query)?
                                        .unwrap_or(self.air[0]);
                                    if self.air.contains(&neighbor) {
                                        exposed = true;
                                        break;
                                    }
                                }
                                if exposed {
                                    continue;
                                }
                            }
                            world
                                .set_section_block(p, *target)
                                .map_err(FeatureExecutionError::Query)?;
                            placed += 1;
                            break;
                        }
                    }
                }
            }
        }
        Ok(placed > 0)
    }
}
fn skip_air_check(random: &mut impl RandomSource, discard: f32) -> bool {
    if discard <= 0.0 {
        true
    } else if discard >= 1.0 {
        false
    } else {
        random.next_float() >= discard
    }
}
