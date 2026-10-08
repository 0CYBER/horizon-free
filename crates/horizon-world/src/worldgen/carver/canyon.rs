use super::*;
use crate::worldgen::int_range;

#[derive(Clone, Debug)]
pub struct CanyonShape {
    pub distance_factor: FloatProvider,
    pub thickness: FloatProvider,
    pub width_smoothness: i32,
    pub horizontal_radius_factor: FloatProvider,
    pub vertical_radius_default_factor: f32,
    pub vertical_radius_center_factor: f32,
    pub y_scale: FloatProvider,
}
#[derive(Clone, Debug)]
pub struct CanyonCarver {
    pub probability: f32,
    pub y: HeightProvider,
    pub vertical_rotation: FloatProvider,
    pub shape: CanyonShape,
}
impl CanyonCarver {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:canyon" {
            return Err(DecodeError::Field("canyon carver type"));
        }
        let probability = scalar(&object, "probability")?;
        if !(0.0..=1.0).contains(&probability) || probability.is_sign_negative() {
            return Err(DecodeError::Field("carver probability"));
        }
        let shape: Object = serde_json::from_str(required(&object, "shape")?.get())?;
        let provider = |name| FloatProvider::from_json(required(&shape, name)?.get());
        Ok(Self {
            probability,
            y: HeightProvider::from_json(required(&object, "y")?.get())?,
            vertical_rotation: FloatProvider::from_json(
                required(&object, "vertical_rotation")?.get(),
            )?,
            shape: CanyonShape {
                distance_factor: provider("distance_factor")?,
                thickness: provider("thickness")?,
                width_smoothness: int_range(
                    required(&shape, "width_smoothness")?,
                    "width_smoothness",
                    1,
                    i32::MAX,
                )?,
                horizontal_radius_factor: provider("horizontal_radius_factor")?,
                vertical_radius_default_factor: scalar(&shape, "vertical_radius_default_factor")?,
                vertical_radius_center_factor: scalar(&shape, "vertical_radius_center_factor")?,
                y_scale: provider("y_scale")?,
            },
        })
    }
    pub fn is_start_chunk(&self, random: &mut impl RandomSource) -> bool {
        random.next_float() <= self.probability
    }
    /// Generate a canyon from one origin chunk. Width factors cover the entire
    /// generation height and use a separate legacy random stream. Output bounds
    /// must lie within that height; paths are bounded to 4096 steps, 4064 width
    /// entries and 2^24 candidate cells. Earlier output survives errors.
    pub fn carve<O: CarverOutput>(
        &self,
        context: WorldGenerationContext,
        random: &mut impl RandomSource,
        target: ChunkPos,
        origin: ChunkPos,
        output: &mut O,
    ) -> Result<bool, CarverError<O::Error>> {
        let mut x = f64::from(
            origin
                .x
                .wrapping_shl(4)
                .wrapping_add(random.next_int_bound(16)?),
        );
        let mut y = f64::from(self.y.sample(random, context)?);
        let mut z = f64::from(
            origin
                .z
                .wrapping_shl(4)
                .wrapping_add(random.next_int_bound(16)?),
        );
        let mut yaw = random.next_float() * std::f32::consts::TAU;
        let mut pitch = self.vertical_rotation.sample(random);
        let y_scale = f64::from(self.shape.y_scale.sample(random));
        let thickness = self.shape.thickness.sample(random);
        let end = (112f32 * self.shape.distance_factor.sample(random)) as i32;
        let mut local = LegacyRandom::new(random.next_long());
        if !(1..=4064).contains(&context.height)
            || output.min_y() < context.min_y
            || i64::from(output.max_y()) > i64::from(context.min_y) + i64::from(context.height)
        {
            return Err(CarverError::InvalidBounds);
        }
        if end > 4096 {
            return Err(CarverError::IterationLimit);
        }
        let mut width = Vec::with_capacity(context.height as usize);
        let mut factor = 1f32;
        for index in 0..context.height {
            if index == 0 || local.next_int_bound(self.shape.width_smoothness)? == 0 {
                factor = 1. + local.next_float() * local.next_float();
            }
            width.push(factor * factor);
        }
        let (mut yaw_change, mut pitch_change) = (0f32, 0f32);
        let mut budget = 1 << 24;
        for step in 0..end {
            let mut horizontal = 1.5
                + f64::from(
                    sin(f64::from(step as f32 * std::f32::consts::PI / end as f32)) * thickness,
                );
            let mut vertical = horizontal * y_scale;
            horizontal *= f64::from(self.shape.horizontal_radius_factor.sample(&mut local));
            let center = 1. - (0.5 - step as f32 / end as f32).abs() * 2.;
            let multiplier = self.shape.vertical_radius_default_factor
                + self.shape.vertical_radius_center_factor * center;
            vertical = (f64::from(multiplier) * vertical)
                * f64::from(local.next_float() * (1. - 0.75) + 0.75);
            let cos_pitch = cos(f64::from(pitch));
            let sin_pitch = sin(f64::from(pitch));
            x += f64::from(cos(f64::from(yaw)) * cos_pitch);
            y += f64::from(sin_pitch);
            z += f64::from(sin(f64::from(yaw)) * cos_pitch);
            pitch *= 0.7;
            pitch += pitch_change * 0.05;
            yaw += yaw_change * 0.05;
            pitch_change *= 0.8;
            yaw_change *= 0.5;
            pitch_change += (local.next_float() - local.next_float()) * local.next_float() * 2.;
            yaw_change += (local.next_float() - local.next_float()) * local.next_float() * 4.;
            if local.next_int_bound(4)? == 0 {
                continue;
            }
            if !can_reach(target, x, z, step, end, thickness) {
                return Ok(true);
            }
            carve_ellipsoid_with_budget(
                target,
                [x, y, z],
                horizontal,
                vertical,
                output,
                |[nx, ny, nz], by| {
                    let factor = width[by.wrapping_sub(context.min_y).wrapping_sub(1) as usize];
                    (nx * nx + nz * nz) * f64::from(factor) + ny * ny / 6. >= 1.
                },
                &mut budget,
            )?;
        }
        Ok(true)
    }
}
fn scalar(object: &Object, name: &'static str) -> Result<f32, DecodeError> {
    number_text(required(object, name)?)?
        .parse()
        .map_err(|_| DecodeError::Field(name))
}
