//! Seeded carver paths using pinned scalar providers and shared mask geometry.
mod canyon;
mod generation;
use super::{
    DecodeError, Object,
    anchor::WorldGenerationContext,
    density::identifier,
    height_provider::HeightProvider,
    number_text, optional, required,
    value_provider::{FloatProvider, IntProvider},
};
pub use canyon::{CanyonCarver, CanyonShape};
pub use generation::{
    CarverCatalog, MaskGenerationError, WorldCarver, generate_carving_mask, large_feature_random,
};
use horizon_core::{
    carving::{CarveError, CarverOutput, can_reach, carve_ellipsoid_with_budget},
    geometry::trig::{cos, sin},
    position::ChunkPos,
    random::{InvalidBound, LegacyRandom, RandomSource},
};

#[derive(Clone, Debug)]
pub struct CaveCarver {
    pub probability: f32,
    pub y: HeightProvider,
    pub count: IntProvider,
    pub thickness: FloatProvider,
    pub weird_thickness_bias: bool,
    pub room_vertical_radius_multiplier: FloatProvider,
    pub horizontal_radius_multiplier: FloatProvider,
    pub vertical_radius_multiplier: FloatProvider,
    pub start_vertical_radius_multiplier: FloatProvider,
    pub floor_level: FloatProvider,
}
#[derive(Debug)]
pub enum CarverError<E> {
    Random(InvalidBound),
    Geometry(CarveError<E>),
    IterationLimit,
    InvalidBounds,
}
impl<E: std::fmt::Display> std::fmt::Display for CarverError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::Geometry(e) => e.fmt(f),
            Self::IterationLimit => f.write_str("carver work limit exceeded"),
            Self::InvalidBounds => f.write_str("carver output or generation bounds are invalid"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CarverError<E> {}
impl<E> From<InvalidBound> for CarverError<E> {
    fn from(e: InvalidBound) -> Self {
        Self::Random(e)
    }
}
impl<E> From<CarveError<E>> for CarverError<E> {
    fn from(e: CarveError<E>) -> Self {
        Self::Geometry(e)
    }
}
impl CaveCarver {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:cave" {
            return Err(DecodeError::Field("cave carver type"));
        }
        let probability: f32 = number_text(required(&object, "probability")?)?
            .parse()
            .map_err(|_| DecodeError::Field("carver probability"))?;
        if !(0.0..=1.0).contains(&probability) || probability.is_sign_negative() {
            return Err(DecodeError::Field("carver probability"));
        }
        let count = IntProvider::from_json(required(&object, "count")?.get())?;
        if count.bounds()[0] < 0 {
            return Err(DecodeError::Field("carver count"));
        }
        let provider = |name| FloatProvider::from_json(required(&object, name)?.get());
        let thickness = provider("thickness")?;
        if thickness.bounds()[0] < 0. {
            return Err(DecodeError::Field("carver thickness"));
        }
        let floor_level = provider("floor_level")?;
        if floor_level.bounds()[0] < -1. || floor_level.bounds()[1] > 1. {
            return Err(DecodeError::Field("carver floor"));
        }
        Ok(Self {
            probability,
            y: HeightProvider::from_json(required(&object, "y")?.get())?,
            count,
            thickness,
            weird_thickness_bias: optional(&object, "weird_thickness_bias")
                .map(|v| serde_json::from_str(v.get()))
                .transpose()?
                .unwrap_or(false),
            room_vertical_radius_multiplier: provider("room_vertical_radius_multiplier")?,
            horizontal_radius_multiplier: provider("horizontal_radius_multiplier")?,
            vertical_radius_multiplier: provider("vertical_radius_multiplier")?,
            start_vertical_radius_multiplier: optional(&object, "start_vertical_radius_multiplier")
                .map(|v| FloatProvider::from_json(v.get()))
                .transpose()?
                .unwrap_or(FloatProvider::Constant(1.)),
            floor_level,
        })
    }
    pub fn is_start_chunk(&self, random: &mut impl RandomSource) -> bool {
        random.next_float() <= self.probability
    }
    /// Carve one origin chunk's paths into the target output. The caller performs
    /// start-chunk selection separately. Local tunnel RNGs always use the legacy
    /// algorithm, regardless of the caller's RNG. At most 4096 starts and 2^24
    /// candidate cells are allowed; errors preserve random draws and prior output.
    pub fn carve<O: CarverOutput>(
        &self,
        context: WorldGenerationContext,
        random: &mut impl RandomSource,
        target: ChunkPos,
        origin: ChunkPos,
        output: &mut O,
    ) -> Result<bool, CarverError<O::Error>> {
        let count = self.count.sample(random)?;
        if count > 4096 {
            return Err(CarverError::IterationLimit);
        }
        let mut budget = 1 << 24;
        for _ in 0..count {
            let x = f64::from(
                origin
                    .x
                    .wrapping_shl(4)
                    .wrapping_add(random.next_int_bound(16)?),
            );
            let y = f64::from(self.y.sample(random, context)?);
            let z = f64::from(
                origin
                    .z
                    .wrapping_shl(4)
                    .wrapping_add(random.next_int_bound(16)?),
            );
            let horizontal = f64::from(self.horizontal_radius_multiplier.sample(random));
            let vertical = f64::from(self.vertical_radius_multiplier.sample(random));
            let start_vertical = f64::from(self.start_vertical_radius_multiplier.sample(random));
            let floor = f64::from(self.floor_level.sample(random));
            let mut run = CaveRun {
                target,
                output,
                budget: &mut budget,
                horizontal,
                vertical,
                floor,
            };
            let mut tunnels = 1;
            if random.next_int_bound(4)? == 0 {
                let multiplier = f64::from(self.room_vertical_radius_multiplier.sample(random));
                let thickness = 1. + random.next_float() * 6.;
                let radius =
                    1.5 + f64::from(sin(f64::from(std::f32::consts::FRAC_PI_2)) * thickness);
                run.ellipsoid([x + 1., y, z], radius, radius * multiplier)?;
                tunnels += random.next_int_bound(4)?;
            }
            for _ in 0..tunnels {
                let yaw = random.next_float() * std::f32::consts::TAU;
                let pitch = (random.next_float() - 0.5) / 4.;
                let mut thickness = self.thickness.sample(random);
                if self.weird_thickness_bias && random.next_int_bound(10)? == 0 {
                    thickness *= random.next_float() * random.next_float() * 3. + 1.;
                }
                let end = 112 - random.next_int_bound(28)?;
                run.tunnel(Tunnel {
                    seed: random.next_long(),
                    position: [x, y, z],
                    thickness,
                    yaw,
                    pitch,
                    start: 0,
                    end,
                    vertical_scale: start_vertical,
                })?;
            }
        }
        Ok(true)
    }
}
struct CaveRun<'a, O: CarverOutput> {
    target: ChunkPos,
    output: &'a mut O,
    budget: &'a mut usize,
    horizontal: f64,
    vertical: f64,
    floor: f64,
}
#[derive(Clone, Copy)]
struct Tunnel {
    seed: i64,
    position: [f64; 3],
    thickness: f32,
    yaw: f32,
    pitch: f32,
    start: i32,
    end: i32,
    vertical_scale: f64,
}
impl<O: CarverOutput> CaveRun<'_, O> {
    fn ellipsoid(
        &mut self,
        position: [f64; 3],
        horizontal: f64,
        vertical: f64,
    ) -> Result<(), CarverError<O::Error>> {
        let floor = self.floor;
        Ok(carve_ellipsoid_with_budget(
            self.target,
            position,
            horizontal,
            vertical,
            self.output,
            |[x, y, z], _| y <= floor || x * x + y * y + z * z >= 1.,
            self.budget,
        )?)
    }
    fn tunnel(&mut self, mut path: Tunnel) -> Result<(), CarverError<O::Error>> {
        let mut random = LegacyRandom::new(path.seed);
        let split = random.next_int_bound(path.end / 2)? + path.end / 4;
        let gentle = random.next_int_bound(6)? == 0;
        let (mut yaw_change, mut pitch_change) = (0f32, 0f32);
        for step in path.start..path.end {
            let radius = 1.5
                + f64::from(
                    sin(f64::from(
                        std::f32::consts::PI * step as f32 / path.end as f32,
                    )) * path.thickness,
                );
            let vertical = radius * path.vertical_scale;
            let cos_pitch = cos(f64::from(path.pitch));
            path.position[0] += f64::from(cos(f64::from(path.yaw)) * cos_pitch);
            path.position[1] += f64::from(sin(f64::from(path.pitch)));
            path.position[2] += f64::from(sin(f64::from(path.yaw)) * cos_pitch);
            path.pitch *= if gentle { 0.92 } else { 0.7 };
            path.pitch += pitch_change * 0.1;
            path.yaw += yaw_change * 0.1;
            pitch_change *= 0.9;
            yaw_change *= 0.75;
            pitch_change += (random.next_float() - random.next_float()) * random.next_float() * 2.;
            yaw_change += (random.next_float() - random.next_float()) * random.next_float() * 4.;
            if step == split && path.thickness > 1. {
                let seed = random.next_long();
                let thickness = random.next_float() * 0.5 + 0.5;
                self.tunnel(Tunnel {
                    seed,
                    thickness,
                    yaw: path.yaw - std::f32::consts::FRAC_PI_2,
                    pitch: path.pitch / 3.,
                    start: step,
                    vertical_scale: 1.,
                    ..path
                })?;
                let seed = random.next_long();
                let thickness = random.next_float() * 0.5 + 0.5;
                self.tunnel(Tunnel {
                    seed,
                    thickness,
                    yaw: path.yaw + std::f32::consts::FRAC_PI_2,
                    pitch: path.pitch / 3.,
                    start: step,
                    vertical_scale: 1.,
                    ..path
                })?;
                return Ok(());
            }
            if random.next_int_bound(4)? == 0 {
                continue;
            }
            if !can_reach(
                self.target,
                path.position[0],
                path.position[2],
                step,
                path.end,
                path.thickness,
            ) {
                return Ok(());
            }
            self.ellipsoid(
                path.position,
                radius * self.horizontal,
                vertical * self.vertical,
            )?;
        }
        Ok(())
    }
}
