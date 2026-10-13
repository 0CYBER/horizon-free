//! Vanilla `minecraft:iceberg`, with native float/double rounding and the
//! short-circuited random draws of IcebergFeature.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    placement::PlacementContext,
    required,
    tree::{TreeResult, kind, read, write},
    tree_blocks::{self as tb, TreeBlocks},
};
use crate::registry::BlockStateId;
use horizon_core::random::RandomSource;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct IcebergFeature {
    state: BlockStateId,
    blocks: Arc<TreeBlocks>,
    air: BlockStateId,
    water: BlockStateId,
    snow_block: BlockStateId,
    ice: BlockStateId,
    packed_ice: BlockStateId,
    blue_ice: BlockStateId,
    snow: BlockStateId,
}
fn ceil(v: f32) -> i32 {
    f64::from(v).ceil() as i32
}
fn pow2(v: i32) -> f64 {
    horizon_core::jmath::pow(f64::from(v), 2.0)
}
impl IcebergFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:iceberg" {
            return Err(DecodeError::Field("iceberg type"));
        }
        let state = bindings
            .codec
            .decode(required(&object, "state")?.get(), false)
            .map_err(|e| DecodeError::Registry(e.to_string()))?;
        let registry = bindings.registry();
        let get = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("missing iceberg block {name}")))
        };
        Ok(Self {
            state,
            blocks: bindings.tree_blocks()?,
            air: get("minecraft:air")?,
            water: get("minecraft:water")?,
            snow_block: get("minecraft:snow_block")?,
            ice: get("minecraft:ice")?,
            packed_ice: get("minecraft:packed_ice")?,
            blue_ice: get("minecraft:blue_ice")?,
            snow: get("minecraft:snow")?,
        })
    }
    fn is(&self, a: BlockStateId, b: BlockStateId) -> bool {
        self.blocks.same_block(a, b)
    }
    fn iceberg_state(&self, s: BlockStateId) -> bool {
        self.is(s, self.packed_ice) || self.is(s, self.snow_block) || self.is(s, self.blue_ice)
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let origin = [origin[0], world.generation().sea_level, origin[2]];
        let snow_on_top = random.next_double() > 0.7;
        let angle = random.next_double() * 2.0 * std::f64::consts::PI;
        let ellipse_a = 11 - random.next_int_bound(5)?;
        let ellipse_c = 3 + random.next_int_bound(3)?;
        let is_ellipse = random.next_double() > 0.7;
        let mut over = if is_ellipse {
            random.next_int_bound(6)? + 6
        } else {
            random.next_int_bound(15)? + 3
        };
        if !is_ellipse && random.next_double() > 0.9 {
            over += random.next_int_bound(19)? + 7;
        }
        let under = (over + random.next_int_bound(11)?).min(18);
        let width = (over + random.next_int_bound(7)? - random.next_int_bound(5)?).min(11);
        let a = if is_ellipse { ellipse_a } else { 11 };
        for xo in -a..a {
            for zo in -a..a {
                for y in 0..over {
                    let radius = if is_ellipse {
                        let scale =
                            (1.0f32 - pow2(y) as f32 / (over as f32 * 1.0f32)) * width as f32;
                        ceil(scale / 2.0)
                    } else {
                        self.radius_round(random, y, over, width)?
                    };
                    if is_ellipse || xo < radius {
                        self.generate_block(
                            world,
                            random,
                            origin,
                            over,
                            xo,
                            y,
                            zo,
                            radius,
                            a,
                            is_ellipse,
                            ellipse_c,
                            angle,
                            snow_on_top,
                        )?;
                    }
                }
            }
        }
        self.smooth(world, origin, width, over, is_ellipse, ellipse_a)?;
        for xo in -a..a {
            for zo in -a..a {
                let mut y = -1;
                while y > -under {
                    let new_a = if is_ellipse {
                        ceil(a as f32 * (1.0f32 - pow2(y) as f32 / (under as f32 * 8.0f32)))
                    } else {
                        a
                    };
                    let radius = self.radius_steep(random, -y, under, width)?;
                    if xo < radius {
                        self.generate_block(
                            world,
                            random,
                            origin,
                            under,
                            xo,
                            y,
                            zo,
                            radius,
                            new_a,
                            is_ellipse,
                            ellipse_c,
                            angle,
                            snow_on_top,
                        )?;
                    }
                    y -= 1;
                }
            }
        }
        let cut_out = if is_ellipse {
            random.next_double() > 0.1
        } else {
            random.next_double() > 0.7
        };
        if cut_out {
            self.cut_out(
                world, random, width, over, origin, is_ellipse, ellipse_a, angle, ellipse_c,
            )?;
        }
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    fn cut_out<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        width: i32,
        height: i32,
        origin: [i32; 3],
        is_ellipse: bool,
        ellipse_a: i32,
        angle: f64,
        ellipse_c: i32,
    ) -> TreeResult<(), W::Error> {
        let sign_x = if random.next_boolean() { -1 } else { 1 };
        let sign_z = if random.next_boolean() { -1 } else { 1 };
        let mut x_off = random.next_int_bound((width / 2 - 2).max(1))?;
        if random.next_boolean() {
            x_off = width / 2 + 1 - random.next_int_bound((width - width / 2 - 1).max(1))?;
        }
        let mut z_off = random.next_int_bound((width / 2 - 2).max(1))?;
        if random.next_boolean() {
            z_off = width / 2 + 1 - random.next_int_bound((width - width / 2 - 1).max(1))?;
        }
        if is_ellipse {
            x_off = random.next_int_bound((ellipse_a - 5).max(1))?;
            z_off = x_off;
        }
        let local = [sign_x * x_off, 0, sign_z * z_off];
        let angle = if is_ellipse {
            angle + std::f64::consts::FRAC_PI_2
        } else {
            random.next_double() * 2.0 * std::f64::consts::PI
        };
        for y in 0..height - 3 {
            let radius = self.radius_round(random, y, height, width)?;
            self.carve(
                world, radius, y, origin, false, angle, local, ellipse_a, ellipse_c,
            )?;
        }
        let mut y = -1;
        while y > -height + random.next_int_bound(5)? {
            let radius = self.radius_steep(random, -y, height, width)?;
            self.carve(
                world, radius, y, origin, true, angle, local, ellipse_a, ellipse_c,
            )?;
            y -= 1;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn carve<W: FeatureWorld>(
        &self,
        world: &mut W,
        radius: i32,
        y: i32,
        origin: [i32; 3],
        under_water: bool,
        angle: f64,
        local: [i32; 3],
        ellipse_a: i32,
        ellipse_c: i32,
    ) -> TreeResult<(), W::Error> {
        let a = radius + 1 + ellipse_a / 3;
        let c = (radius - 3).min(3) + ellipse_c / 2 - 1;
        for xo in -a..a {
            for zo in -a..a {
                if signed_ellipse(xo, zo, local, a, c, angle) < 0.0 {
                    let p = tb::offset(origin, xo, y, zo);
                    let state = read(world, p)?;
                    if self.iceberg_state(state) || self.is(state, self.snow_block) {
                        if under_water {
                            write(world, p, self.water, 3)?;
                        } else {
                            write(world, p, self.air, 3)?;
                            let above = tb::offset(p, 0, 1, 0);
                            if self.is(read(world, above)?, self.snow) {
                                write(world, above, self.air, 3)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn generate_block<W: FeatureWorld, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        height: i32,
        xo: i32,
        y: i32,
        zo: i32,
        radius: i32,
        a: i32,
        is_ellipse: bool,
        ellipse_c: i32,
        angle: f64,
        snow_on_top: bool,
    ) -> TreeResult<(), W::Error> {
        let signed = if is_ellipse {
            let mut c = ellipse_c;
            if y > 0 && height - y <= 3 {
                c -= 4 - (height - y);
            }
            signed_ellipse(xo, zo, [0; 3], a, c, angle)
        } else {
            let off = 10.0f32 * random.next_float().clamp(0.2, 0.8) / radius as f32;
            f64::from(off) + pow2(xo) + pow2(zo) - pow2(radius)
        };
        // `signedDist < 0.0` fails for NaN too (a zero-width ellipse layer at its centre).
        if signed.is_nan() || signed >= 0.0 {
            return Ok(());
        }
        let p = tb::offset(origin, xo, y, zo);
        let compare = if is_ellipse {
            -0.5
        } else {
            f64::from(-6 - random.next_int_bound(3)?)
        };
        if signed > compare && random.next_double() > 0.9 {
            return Ok(());
        }
        let h_diff = height - y;
        let state = read(world, p)?;
        if self.blocks.is_air(state)
            || self.is(state, self.snow_block)
            || self.is(state, self.ice)
            || self.is(state, self.water)
        {
            let randomness = !is_ellipse || random.next_double() > 0.05;
            let divisor = if is_ellipse { 3 } else { 2 };
            let snow = snow_on_top
                && !self.is(state, self.water)
                && f64::from(h_diff)
                    <= f64::from(random.next_int_bound((height / divisor).max(1))?)
                        + f64::from(height) * 0.6
                && randomness;
            write(world, p, if snow { self.snow_block } else { self.state }, 3)?;
        }
        Ok(())
    }
    fn radius_round<R: RandomSource, E>(
        &self,
        random: &mut R,
        y: i32,
        height: i32,
        width: i32,
    ) -> TreeResult<i32, E> {
        let k = 3.5f32 - random.next_float();
        let mut scale = (1.0f32 - pow2(y) as f32 / (height as f32 * k)) * width as f32;
        if height > 15 + random.next_int_bound(5)? {
            let temp = if y < 3 + random.next_int_bound(6)? {
                y / 2
            } else {
                y
            };
            scale = (1.0f32 - temp as f32 / (height as f32 * k * 0.4f32)) * width as f32;
        }
        Ok(ceil(scale / 2.0))
    }
    fn radius_steep<R: RandomSource, E>(
        &self,
        random: &mut R,
        y: i32,
        height: i32,
        width: i32,
    ) -> TreeResult<i32, E> {
        let k = 1.0f32 + random.next_float() / 2.0;
        let scale = (1.0f32 - y as f32 / (height as f32 * k)) * width as f32;
        Ok(ceil(scale / 2.0))
    }
    fn smooth<W: FeatureWorld>(
        &self,
        world: &mut W,
        origin: [i32; 3],
        width: i32,
        height: i32,
        is_ellipse: bool,
        ellipse_a: i32,
    ) -> TreeResult<(), W::Error> {
        let a = if is_ellipse { ellipse_a } else { width / 2 };
        for x in -a..=a {
            for z in -a..=a {
                for y in 0..=height {
                    let p = tb::offset(origin, x, y, z);
                    let state = read(world, p)?;
                    if !(self.iceberg_state(state) || self.is(state, self.snow)) {
                        continue;
                    }
                    if self.blocks.is_air(read(world, tb::offset(p, 0, -1, 0))?) {
                        write(world, p, self.air, 3)?;
                        write(world, tb::offset(p, 0, 1, 0), self.air, 3)?;
                    } else if self.iceberg_state(state) {
                        let mut counter = 0;
                        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                            if !self.iceberg_state(read(world, tb::offset(p, dx, 0, dz))?) {
                                counter += 1;
                            }
                        }
                        if counter >= 3 {
                            write(world, p, self.air, 3)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
fn signed_ellipse(xo: i32, zo: i32, origin: [i32; 3], a: i32, c: i32, angle: f64) -> f64 {
    let dx = f64::from(xo - origin[0]);
    let dz = f64::from(zo - origin[2]);
    let (sin, cos) = (
        horizon_core::jmath::sin(angle),
        horizon_core::jmath::cos(angle),
    );
    horizon_core::jmath::pow((dx * cos - dz * sin) / f64::from(a), 2.0)
        + horizon_core::jmath::pow((dx * sin + dz * cos) / f64::from(c), 2.0)
        - 1.0
}
