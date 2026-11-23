use super::{DensityError, DensityVolume, NanRange, PerlinNoise, SimplexNoise, SmearedPerlinNoise};
use std::any::Any;
use std::sync::Arc;

/// Immutable noise sampling. Implementations may override the default scalar
/// volume traversal when vanilla has a distinct bulk arithmetic path.
pub trait Noise: Any + std::fmt::Debug + Send + Sync {
    fn range(&self) -> Result<[f32; 2], NanRange>;
    fn get2(&self, x: f64, z: f64) -> f32;
    fn get3(&self, x: f64, y: f64, z: f64) -> f32;
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [sx, sy, sz] = volume.sizes();
        let mut index = 0;
        for z in 0..sz {
            let z = f64::from(volume.coordinate(2, z)) * xz_scale;
            for x in 0..sx {
                let x = f64::from(volume.coordinate(0, x)) * xz_scale;
                for y in 0..sy {
                    let y = f64::from(volume.coordinate(1, y)) * y_scale;
                    buffer[index] += amplitude * self.get3(x, y, z);
                    index += 1;
                }
            }
        }
        Ok(())
    }
    /// [`Noise::add_to_volume`] for a caller that reads only the Ys set in `needed`: bit `y` of
    /// `needed[column]`, X/Z columns in buffer order. The other Ys may be left unchanged.
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        _needed: &[u64],
    ) -> Result<(), DensityError> {
        self.add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)
    }
}

impl Noise for PerlinNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(PerlinNoise::range(self))
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        PerlinNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        PerlinNoise::get3(self, x, y, z)
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        PerlinNoise::add_to_volume(self, buffer, volume, xz_scale, y_scale, amplitude)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        PerlinNoise::add_to_needed(self, buffer, volume, xz_scale, y_scale, amplitude, needed)
    }
}
impl Noise for SmearedPerlinNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        SmearedPerlinNoise::range(self)
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        SmearedPerlinNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        SmearedPerlinNoise::get3(self, x, y, z)
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        SmearedPerlinNoise::add_to_volume(self, buffer, volume, xz_scale, y_scale, amplitude)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        SmearedPerlinNoise::add_to_needed(
            self, buffer, volume, xz_scale, y_scale, amplitude, needed,
        )
    }
}
impl Noise for SimplexNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(SimplexNoise::range(self))
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        SimplexNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        SimplexNoise::get3(self, x, y, z)
    }
}

#[derive(Clone, Debug)]
struct TypedLayer<T> {
    noise: Arc<T>,
    frequency: f64,
    amplitude: f32,
}

/// Homogeneous devirtualized layer storage for stacks whose layers are all a
/// single concrete `PerlinNoise`, `SmearedPerlinNoise`, or `SimplexNoise`.
///
/// Concrete layers share the existing `Arc<T>` instances from `layers` via
/// `Arc::downcast::<T>()` — zero table cloning and no duplicate sampler tables
/// alongside `layers` (only an `Arc` refcount bump per layer). Mixed stacks and
/// custom wrapper `Noise` implementations fall back to `FastLayers::Dynamic`.
#[derive(Clone, Debug)]
enum FastLayers {
    Perlin(Box<[TypedLayer<PerlinNoise>]>),
    SmearedPerlin(Box<[TypedLayer<SmearedPerlinNoise>]>),
    Simplex(Box<[TypedLayer<SimplexNoise>]>),
    Dynamic,
}

/// Downcasts every layer to concrete type `T` using non-overridable [`Any`]
/// type identity, sharing the existing [`Arc`] allocations with zero cloning.
fn try_downcast_layers<T: Any + Send + Sync>(
    layers: &[NoiseLayer],
) -> Option<Box<[TypedLayer<T>]>> {
    if !layers.iter().all(|layer| {
        let any: &dyn Any = layer.noise.as_ref();
        any.is::<T>()
    }) {
        return None;
    }
    layers
        .iter()
        .map(|layer| {
            let cloned: Arc<dyn Noise> = Arc::clone(&layer.noise);
            let any: Arc<dyn Any + Send + Sync> = cloned;
            let noise = any.downcast::<T>().ok()?;
            Some(TypedLayer {
                noise,
                frequency: layer.frequency,
                amplitude: layer.amplitude,
            })
        })
        .collect::<Option<Box<[_]>>>()
}

#[derive(Clone, Debug)]
pub struct NoiseLayer {
    pub noise: Arc<dyn Noise>,
    pub frequency: f64,
    pub amplitude: f32,
}

#[derive(Clone, Debug)]
pub struct NoiseStack {
    layers: Vec<NoiseLayer>,
    fast: FastLayers,
    y0_fast_path: bool,
    range: [f32; 2],
}

impl NoiseStack {
    pub fn builder() -> NoiseStackBuilder {
        NoiseStackBuilder::default()
    }
    pub fn layers(&self) -> &[NoiseLayer] {
        &self.layers
    }
    pub fn get_layer(&self, index: usize) -> Option<&Arc<dyn Noise>> {
        self.layers.get(index).map(|layer| &layer.noise)
    }
}
impl Noise for NoiseStack {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(self.range)
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        let mut value = 0.0;
        match &self.fast {
            FastLayers::Perlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
        }
        value
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut value = 0.0;
        match &self.fast {
            FastLayers::Perlin(layers) => {
                if y == 0.0 && self.y0_fast_path {
                    for layer in layers {
                        value += layer.amplitude
                            * layer
                                .noise
                                .get3_y0(x * layer.frequency, z * layer.frequency);
                    }
                } else {
                    for layer in layers {
                        value += layer.amplitude
                            * layer.noise.get3(
                                x * layer.frequency,
                                y * layer.frequency,
                                z * layer.frequency,
                            );
                    }
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
        }
        value
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        self.add_layers(buffer, volume, [xz_scale, y_scale], amplitude, None)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        self.add_layers(buffer, volume, [xz_scale, y_scale], amplitude, Some(needed))
    }
}
impl NoiseStack {
    fn add_layers(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        [xz_scale, y_scale]: [f64; 2],
        amplitude: f32,
        needed: Option<&[u64]>,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        match &self.fast {
            FastLayers::Perlin(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    Noise::add_to_volume(
                        layer.noise.as_ref(),
                        buffer,
                        volume,
                        xz_scale,
                        y_scale,
                        amplitude,
                    )?;
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoiseStackBuilder {
    layers: Vec<NoiseLayer>,
}
impl NoiseStackBuilder {
    pub fn add(&mut self, noise: Arc<dyn Noise>, frequency: f64, amplitude: f32) -> &mut Self {
        self.layers.push(NoiseLayer {
            noise,
            frequency,
            amplitude,
        });
        self
    }
    /// Flattening rescales each existing layer before subsequent sampling.
    pub fn add_stack(&mut self, stack: &NoiseStack, frequency: f64, amplitude: f32) -> &mut Self {
        self.layers
            .extend(stack.layers.iter().map(|layer| NoiseLayer {
                noise: Arc::clone(&layer.noise),
                frequency: layer.frequency * frequency,
                amplitude: layer.amplitude * amplitude,
            }));
        self
    }
    pub fn build(&self) -> Result<NoiseStack, NanRange> {
        let mut range = [0.0; 2];
        for layer in &self.layers {
            let bounds = layer.noise.range()?;
            if layer.amplitude.is_nan() {
                return Err(NanRange);
            }
            let scaled = if bounds[0].is_nan() || bounds[1].is_nan() {
                [f32::NAN; 2]
            } else {
                // Interval.mulBound defines zero times infinity as positive zero.
                let product = |v: f32| {
                    if v == 0.0 || layer.amplitude == 0.0 {
                        0.0
                    } else {
                        v * layer.amplitude
                    }
                };
                let a = product(bounds[0]);
                let b = product(bounds[1]);
                if a < b {
                    [a, b]
                } else if a > b {
                    [b, a]
                } else if a == 0.0 {
                    [
                        f32::from_bits(a.to_bits() | b.to_bits()),
                        f32::from_bits(a.to_bits() & b.to_bits()),
                    ]
                } else {
                    [a, b]
                }
            };
            range = [range[0] + scaled[0], range[1] + scaled[1]];
            if range[0].is_nan() || range[1].is_nan() {
                range = [f32::NAN; 2];
            }
            // Interval.of interns [0,1], including a supplied negative zero.
            if range[0] == 0.0 && range[1] == 1.0 {
                range[0] = 0.0;
            }
        }
        let fast = if self.layers.is_empty() {
            FastLayers::Perlin(Box::new([]))
        } else if let Some(layers) = try_downcast_layers::<PerlinNoise>(&self.layers) {
            FastLayers::Perlin(layers)
        } else if let Some(layers) = try_downcast_layers::<SmearedPerlinNoise>(&self.layers) {
            FastLayers::SmearedPerlin(layers)
        } else if let Some(layers) = try_downcast_layers::<SimplexNoise>(&self.layers) {
            FastLayers::Simplex(layers)
        } else {
            FastLayers::Dynamic
        };
        let y0_fast_path = match &fast {
            FastLayers::Perlin(layers) => layers.iter().all(|layer| {
                layer.frequency.is_finite()
                    && layer.noise.table().offsets()[1].to_bits() != (-0.0_f64).to_bits()
            }),
            _ => false,
        };
        Ok(NoiseStack {
            layers: self.layers.clone(),
            fast,
            y0_fast_path,
            range,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::LegacyRandom;

    #[derive(Debug)]
    struct OffsetPerlin {
        inner: PerlinNoise,
        offset: f32,
    }

    impl Noise for OffsetPerlin {
        fn range(&self) -> Result<[f32; 2], NanRange> {
            let [lo, hi] = self.inner.range();
            Ok([lo + self.offset, hi + self.offset])
        }
        fn get2(&self, x: f64, z: f64) -> f32 {
            self.inner.get2(x, z) + self.offset
        }
        fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
            self.inner.get3(x, y, z) + self.offset
        }
    }

    #[derive(Debug)]
    struct OffsetSmeared {
        inner: SmearedPerlinNoise,
        offset: f32,
    }

    impl Noise for OffsetSmeared {
        fn range(&self) -> Result<[f32; 2], NanRange> {
            let [lo, hi] = self.inner.range()?;
            Ok([lo + self.offset, hi + self.offset])
        }
        fn get2(&self, x: f64, z: f64) -> f32 {
            self.inner.get2(x, z) + self.offset
        }
        fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
            self.inner.get3(x, y, z) + self.offset
        }
    }

    #[derive(Debug)]
    struct OffsetSimplex {
        inner: SimplexNoise,
        offset: f32,
    }

    impl Noise for OffsetSimplex {
        fn range(&self) -> Result<[f32; 2], NanRange> {
            let [lo, hi] = self.inner.range();
            Ok([lo + self.offset, hi + self.offset])
        }
        fn get2(&self, x: f64, z: f64) -> f32 {
            self.inner.get2(x, z) + self.offset
        }
        fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
            self.inner.get3(x, y, z) + self.offset
        }
    }

    #[test]
    fn custom_wrapper_around_concrete_samplers_is_never_bypassed() {
        let perlin = PerlinNoise::new(&mut LegacyRandom::new(0));
        let smeared = SmearedPerlinNoise::new(&mut LegacyRandom::new(1), 0.125);
        let simplex = SimplexNoise::new(&mut LegacyRandom::new(2));

        let wrappers: Vec<Arc<dyn Noise>> = vec![
            Arc::new(OffsetPerlin {
                inner: perlin.clone(),
                offset: 3.0,
            }),
            Arc::new(OffsetSmeared {
                inner: smeared,
                offset: 3.0,
            }),
            Arc::new(OffsetSimplex {
                inner: simplex,
                offset: 3.0,
            }),
        ];

        let volume = DensityVolume::new([3, 4, 2], [-5, 10, 7], [2, 1, 3]).unwrap();
        let needed = [0b1010_u64, 0, u64::MAX, 0b0101, 0b1111, 0b0001];

        for wrapper in &wrappers {
            let freq = 0.5_f64;
            let amp = 0.75_f32;
            let mut builder = NoiseStack::builder();
            builder.add(Arc::clone(wrapper), freq, amp);
            let stack = builder.build().unwrap();

            assert!(matches!(stack.fast, FastLayers::Dynamic));
            assert!(!stack.y0_fast_path);
            assert!(Arc::ptr_eq(stack.get_layer(0).unwrap(), wrapper));

            let [lo, hi] = wrapper.range().unwrap();
            assert_eq!(stack.range().unwrap(), [lo * amp, hi * amp]);

            for (x, y, z) in [
                (0.0, 0.0, 0.0),
                (1.25, -0.0, -3.5),
                (-7.5, 2.25, 4.75),
                (16_777_216.0, 0.0, -16_777_216.0),
            ] {
                assert_eq!(
                    stack.get2(x, z).to_bits(),
                    (0.0_f32 + amp * wrapper.get2(x * freq, z * freq)).to_bits()
                );
                assert_eq!(
                    stack.get3(x, y, z).to_bits(),
                    (0.0_f32 + amp * wrapper.get3(x * freq, y * freq, z * freq)).to_bits()
                );
            }

            let mut expected = vec![0.5_f32; volume.size()];
            let mut actual = expected.clone();
            wrapper
                .add_to_volume(&mut expected, volume, 0.25 * freq, 0.5 * freq, 1.5 * amp)
                .unwrap();
            stack
                .add_to_volume(&mut actual, volume, 0.25, 0.5, 1.5)
                .unwrap();
            assert_eq!(
                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );

            let mut actual_needed = vec![0.5_f32; volume.size()];
            stack
                .add_to_needed(&mut actual_needed, volume, 0.25, 0.5, 1.5, &needed)
                .unwrap();
            assert_eq!(
                actual_needed
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }

        // Explicitly verify CodeRabbit's counterexample: at an exact lattice corner where
        // inner PerlinNoise returns 0.0, the wrapper stack must return 3.0, never 0.0.
        {
            let [ox, oy, oz] = *perlin.table().offsets();
            let (zx, zy, zz) = (-ox, -oy, -oz);
            assert_eq!(perlin.get3(zx, zy, zz), 0.0);
            let mut b = NoiseStack::builder();
            b.add(Arc::clone(&wrappers[0]), 1.0, 1.0);
            let s = b.build().unwrap();
            assert_eq!(s.get3(zx, zy, zz).to_bits(), 3.0_f32.to_bits());
        }

        // Mixing a raw PerlinNoise with a wrapped OffsetPerlin must also stay Dynamic.
        let mut mixed_builder = NoiseStack::builder();
        mixed_builder.add(Arc::new(perlin), 1.0, 1.0);
        mixed_builder.add(Arc::clone(&wrappers[0]), 0.5, 1.0);
        let mixed = mixed_builder.build().unwrap();
        assert!(matches!(mixed.fast, FastLayers::Dynamic));
        assert!(!mixed.y0_fast_path);

        // Nesting a homogeneous NoiseStack via `add` (without flattening) must stay Dynamic.
        let mut inner_builder = NoiseStack::builder();
        inner_builder.add(
            Arc::new(PerlinNoise::new(&mut LegacyRandom::new(9))),
            1.0,
            1.0,
        );
        let inner_stack = inner_builder.build().unwrap();
        assert!(matches!(inner_stack.fast, FastLayers::Perlin(_)));
        let mut outer_builder = NoiseStack::builder();
        outer_builder.add(Arc::new(inner_stack), 2.0, 0.5);
        let outer = outer_builder.build().unwrap();
        assert!(matches!(outer.fast, FastLayers::Dynamic));
    }

    #[test]
    fn homogeneous_fast_layers_share_arcs_without_cloning() {
        let p0 = Arc::new(PerlinNoise::new(&mut LegacyRandom::new(0)));
        let p1 = Arc::new(PerlinNoise::new(&mut LegacyRandom::new(1)));
        let p0_dyn: Arc<dyn Noise> = Arc::clone(&p0) as Arc<dyn Noise>;
        let p1_dyn: Arc<dyn Noise> = Arc::clone(&p1) as Arc<dyn Noise>;
        let mut pb = NoiseStack::builder();
        pb.add(Arc::clone(&p0_dyn), 1.0, 0.5)
            .add(Arc::clone(&p1_dyn), 2.0, 0.25);
        let p_stack = pb.build().unwrap();
        assert!(p_stack.y0_fast_path);
        match &p_stack.fast {
            FastLayers::Perlin(layers) => {
                assert_eq!(layers.len(), 2);
                assert!(Arc::ptr_eq(&layers[0].noise, &p0));
                assert!(Arc::ptr_eq(&layers[1].noise, &p1));
            }
            other => panic!("expected FastLayers::Perlin, got {other:?}"),
        }

        // Flattening via `add_stack` must keep sharing the exact same Arc allocations.
        let mut flat_builder = NoiseStack::builder();
        flat_builder.add_stack(&p_stack, 0.5, 2.0);
        let flat = flat_builder.build().unwrap();
        match &flat.fast {
            FastLayers::Perlin(layers) => {
                assert_eq!(layers.len(), 2);
                assert!(Arc::ptr_eq(&layers[0].noise, &p0));
                assert!(Arc::ptr_eq(&layers[1].noise, &p1));
            }
            other => panic!("expected FastLayers::Perlin after add_stack, got {other:?}"),
        }

        let sm = Arc::new(SmearedPerlinNoise::new(&mut LegacyRandom::new(2), 0.25));
        let sm_dyn: Arc<dyn Noise> = Arc::clone(&sm) as Arc<dyn Noise>;
        let mut sb = NoiseStack::builder();
        sb.add(sm_dyn, 1.0, 1.0);
        let sm_stack = sb.build().unwrap();
        assert!(!sm_stack.y0_fast_path);
        match &sm_stack.fast {
            FastLayers::SmearedPerlin(layers) => {
                assert_eq!(layers.len(), 1);
                assert!(Arc::ptr_eq(&layers[0].noise, &sm));
            }
            other => panic!("expected FastLayers::SmearedPerlin, got {other:?}"),
        }

        let sx = Arc::new(SimplexNoise::new(&mut LegacyRandom::new(3)));
        let sx_dyn: Arc<dyn Noise> = Arc::clone(&sx) as Arc<dyn Noise>;
        let mut xb = NoiseStack::builder();
        xb.add(sx_dyn, 1.0, 1.0);
        let sx_stack = xb.build().unwrap();
        assert!(!sx_stack.y0_fast_path);
        match &sx_stack.fast {
            FastLayers::Simplex(layers) => {
                assert_eq!(layers.len(), 1);
                assert!(Arc::ptr_eq(&layers[0].noise, &sx));
            }
            other => panic!("expected FastLayers::Simplex, got {other:?}"),
        }

        // Mixed concrete samplers stay on Dynamic dispatch.
        let mut mixed = NoiseStack::builder();
        mixed.add(Arc::clone(&p0_dyn), 1.0, 1.0).add(
            Arc::new(SimplexNoise::new(&mut LegacyRandom::new(4))),
            1.0,
            1.0,
        );
        let mixed_stack = mixed.build().unwrap();
        assert!(matches!(mixed_stack.fast, FastLayers::Dynamic));
        assert!(!mixed_stack.y0_fast_path);

        // Non-finite frequencies must disable `y0_fast_path` so `0.0 * inf` / `0.0 * NaN` yields NaN.
        for freq in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let mut b = NoiseStack::builder();
            b.add(Arc::clone(&p0_dyn), freq, 1.0);
            let stack = b.build().unwrap();
            assert!(matches!(stack.fast, FastLayers::Perlin(_)));
            assert!(!stack.y0_fast_path);
            assert!(stack.get3(1.0, 0.0, 2.0).is_nan());
            assert!(stack.get3(1.0, -0.0, 2.0).is_nan());
        }
    }
}
