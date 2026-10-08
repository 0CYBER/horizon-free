use super::{NanRange, NoiseStack, PerlinNoise};
use crate::random::{PositionalRandom, RandomSource};
use std::sync::Arc;

/// An allocation bound for the builder API (the vanilla data codec allows 32).
const MAX_BUILDER_OCTAVES: i32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseBuildError {
    NegativeOctaveCount,
    TooManyOctaves,
    CountAfterModifiers,
    ModifierIndex,
    EmptyAmplitudes,
    NanRange,
    InvertedRange,
    MissingNoiseLevels,
    PositiveLegacyOctaves,
    LegacyWorkLimit,
}
impl std::fmt::Display for NoiseBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NegativeOctaveCount => "octave count cannot be negative",
            Self::TooManyOctaves => "octave count exceeds the 4096-layer allocation bound",
            Self::CountAfterModifiers => {
                "cannot change octave count after setting amplitude modifiers"
            }
            Self::ModifierIndex => "amplitude modifier index is outside the octave range",
            Self::EmptyAmplitudes => "at least one amplitude is required",
            Self::NanRange => "noise interval bounds cannot include NaN",
            Self::InvertedRange => "noise interval lower bound exceeds its upper bound",
            Self::MissingNoiseLevels => "legacy initializer could not create all nonzero octaves",
            Self::PositiveLegacyOctaves => "positive legacy octaves are disabled",
            Self::LegacyWorkLimit => "legacy initializer exceeds the 4096-octave work bound",
        })
    }
}
impl std::error::Error for NoiseBuildError {}
impl From<NanRange> for NoiseBuildError {
    fn from(_: NanRange) -> Self {
        Self::NanRange
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Enabled,
    Disabled,
    Legacy,
}

#[derive(Clone, Copy, Debug)]
pub struct OctaveInfo {
    pub octave_index: i32,
    pub frequency: f64,
    pub amplitude: f64,
}
impl OctaveInfo {
    pub fn seed(&self) -> String {
        format!("octave_{}", self.octave_index)
    }
}

#[derive(Clone, Debug)]
pub struct NormalNoise {
    parameters: NormalNoiseBuilder,
    octaves: Vec<OctaveInfo>,
    normalization_factor: f64,
    range: [f32; 2],
}
impl NormalNoise {
    pub fn builder() -> NormalNoiseBuilder {
        NormalNoiseBuilder::default()
    }
    pub fn octaves(&self) -> &[OctaveInfo] {
        &self.octaves
    }
    pub fn normalization_factor(&self) -> f64 {
        self.normalization_factor
    }
    pub fn range(&self) -> [f32; 2] {
        self.range
    }

    pub fn create(&self, random: &mut impl RandomSource) -> Result<NoiseStack, NoiseBuildError> {
        // Even an empty stack consumes both positional forks.
        let first = random.fork_positional();
        let second = random.fork_positional();
        let mut builder = NoiseStack::builder();
        for octave in &self.octaves {
            let name = octave.seed();
            let a = PerlinNoise::new(&mut first.from_hash_of(&name));
            let b = PerlinNoise::new(&mut second.from_hash_of(&name));
            let amplitude = (self.normalization_factor * octave.amplitude) as f32;
            builder.add(Arc::new(a), octave.frequency, amplitude);
            builder.add(
                Arc::new(b),
                octave.frequency * 1.018_126_888_217_522_7,
                amplitude,
            );
        }
        Ok(builder.build()?)
    }

    pub fn create_for_legacy_nether_biome(
        &self,
        random: &mut impl RandomSource,
    ) -> Result<NoiseStack, NoiseBuildError> {
        let defaults;
        let modifiers = if self.parameters.modifiers.is_empty() {
            defaults = vec![1.0; self.parameters.octave_count as usize];
            &defaults
        } else {
            &self.parameters.modifiers
        };
        let first = create_legacy_fbm(random, self.parameters.base_octave, modifiers)?;
        let second = create_legacy_fbm(random, self.parameters.base_octave, modifiers)?;
        let amplitude = (self.normalization_factor * self.parameters.base_amplitude) as f32;
        Ok(NoiseStack::builder()
            .add_stack(&first, 1.0, amplitude)
            .add_stack(&second, 1.018_126_888_217_522_7, amplitude)
            .build()?)
    }

    pub fn create_parity(base_octave: i32, amplitudes: &[f64]) -> Result<Self, NoiseBuildError> {
        if amplitudes.is_empty() {
            return Err(NoiseBuildError::EmptyAmplitudes);
        }
        if amplitudes.len() > MAX_BUILDER_OCTAVES as usize {
            return Err(NoiseBuildError::TooManyOctaves);
        }
        let count = amplitudes.len() as i32;
        let octaves = build_octaves(base_octave, 1.0, count, true, amplitudes);
        let factor = normalization_factor(sum_abs(&octaves), &octaves);
        let base_amplitude = if factor == 0.0 {
            1.0
        } else {
            parity_factor(1.0, count, amplitudes) / factor
        };
        NormalNoiseBuilder {
            base_octave,
            base_amplitude,
            octave_count: count,
            normalization: Normalization::Enabled,
            modifiers: amplitudes.to_vec(),
        }
        .build()
    }
}

#[derive(Clone, Debug)]
pub struct NormalNoiseBuilder {
    base_amplitude: f64,
    base_octave: i32,
    octave_count: i32,
    normalization: Normalization,
    modifiers: Vec<f64>,
}
impl Default for NormalNoiseBuilder {
    fn default() -> Self {
        Self {
            base_amplitude: 1.0,
            base_octave: 0,
            octave_count: 1,
            normalization: Normalization::Enabled,
            modifiers: Vec::new(),
        }
    }
}
impl NormalNoiseBuilder {
    pub fn set_base_amplitude(&mut self, amplitude: f64) -> &mut Self {
        self.base_amplitude = amplitude;
        self
    }
    pub fn set_base_octave(&mut self, octave: i32) -> &mut Self {
        self.base_octave = octave;
        self
    }
    pub fn set_normalization(&mut self, normalization: Normalization) -> &mut Self {
        self.normalization = normalization;
        self
    }
    pub fn set_octave_count(&mut self, count: i32) -> Result<&mut Self, NoiseBuildError> {
        if !self.modifiers.is_empty() {
            return Err(NoiseBuildError::CountAfterModifiers);
        }
        self.octave_count = count;
        Ok(self)
    }
    pub fn set_amplitude_modifier(
        &mut self,
        index: i32,
        value: f64,
    ) -> Result<&mut Self, NoiseBuildError> {
        if index < 0 || index >= self.octave_count {
            return Err(NoiseBuildError::ModifierIndex);
        }
        if self.octave_count > MAX_BUILDER_OCTAVES {
            return Err(NoiseBuildError::TooManyOctaves);
        }
        if self.modifiers.is_empty() {
            self.modifiers.resize(self.octave_count as usize, 1.0);
        }
        self.modifiers[index as usize] = value;
        Ok(self)
    }
    pub fn build(&self) -> Result<NormalNoise, NoiseBuildError> {
        if self.octave_count < 0 {
            return Err(NoiseBuildError::NegativeOctaveCount);
        }
        if self.octave_count > MAX_BUILDER_OCTAVES {
            return Err(NoiseBuildError::TooManyOctaves);
        }
        let octaves = build_octaves(
            self.base_octave,
            self.base_amplitude,
            self.octave_count,
            self.normalization != Normalization::Disabled,
            &self.modifiers,
        );
        let mut sum = sum_abs(&octaves);
        let mut factor = normalization_factor(sum, &octaves);
        if self.normalization == Normalization::Legacy && factor != 0.0 {
            let legacy = parity_factor(self.base_amplitude, self.octave_count, &self.modifiers);
            sum *= legacy / factor;
            factor = legacy;
        }
        let bound = (sum * (1.0 / 3.0) * 6.0) as f32;
        if bound.is_nan() {
            return Err(NoiseBuildError::NanRange);
        }
        if bound < 0.0 {
            return Err(NoiseBuildError::InvertedRange);
        }
        Ok(NormalNoise {
            parameters: self.clone(),
            octaves,
            normalization_factor: factor,
            range: [-bound, bound],
        })
    }
}

/// The legacy initializer consumes the caller's stream in reverse octave order.
/// Missing and zero-amplitude octaves consume exactly 262 integer draws.
/// Requests outside the documented allocation/work bounds fail before drawing.
pub fn create_legacy_fbm(
    random: &mut impl RandomSource,
    base_octave: i32,
    amplitudes: &[f64],
) -> Result<NoiseStack, NoiseBuildError> {
    if amplitudes.len() > MAX_BUILDER_OCTAVES as usize {
        return Err(NoiseBuildError::TooManyOctaves);
    }
    let zero_index = base_octave.wrapping_neg();
    if zero_index.wrapping_sub(1) >= MAX_BUILDER_OCTAVES {
        return Err(NoiseBuildError::LegacyWorkLimit);
    }
    let count = amplitudes.len() as i32;
    let mut levels = vec![None; amplitudes.len()];
    let zero_noise = PerlinNoise::new(random);
    if zero_index >= 0 && zero_index < count && amplitudes[zero_index as usize] != 0.0 {
        levels[zero_index as usize] = Some(zero_noise);
    }
    for index in (0..zero_index).rev() {
        if index < count && amplitudes[index as usize] != 0.0 {
            levels[index as usize] = Some(PerlinNoise::new(random));
        } else {
            random.consume_count(262);
        }
    }
    if levels.iter().filter(|v| v.is_some()).count()
        != amplitudes.iter().filter(|v| **v != 0.0).count()
    {
        return Err(NoiseBuildError::MissingNoiseLevels);
    }
    if zero_index < count - 1 {
        return Err(NoiseBuildError::PositiveLegacyOctaves);
    }
    let mut frequency = power_of_two(zero_index.wrapping_neg());
    let mut amplitude = power_of_two(count - 1) / (power_of_two(count) - 1.0);
    let mut builder = NoiseStack::builder();
    for (index, noise) in levels.into_iter().enumerate() {
        if let Some(noise) = noise {
            builder.add(
                Arc::new(noise),
                frequency,
                (amplitude * amplitudes[index]) as f32,
            );
        }
        frequency *= 2.0;
        amplitude /= 2.0;
    }
    Ok(builder.build()?)
}

fn modifier(modifiers: &[f64], index: usize) -> f64 {
    if modifiers.is_empty() {
        1.0
    } else {
        modifiers[index]
    }
}

fn build_octaves(
    base: i32,
    amplitude: f64,
    count: i32,
    normalize: bool,
    modifiers: &[f64],
) -> Vec<OctaveInfo> {
    let mut frequency = power_of_two(base);
    let mut amplitude = amplitude;
    if normalize {
        amplitude *= power_of_two(count - 1) / (power_of_two(count) - 1.0);
    }
    let mut result = Vec::with_capacity(count as usize);
    for index in 0..count {
        let multiplier = modifier(modifiers, index as usize);
        if multiplier != 0.0 {
            result.push(OctaveInfo {
                octave_index: base.wrapping_add(index),
                frequency,
                amplitude: amplitude * multiplier,
            });
        }
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    result
}

// Math.pow(2, integer), including exact subnormal powers and saturation, without
// a platform libm approximation. Frequencies subsequently double iteratively.
fn power_of_two(exponent: i32) -> f64 {
    match exponent {
        ..=-1075 => 0.0,
        -1074..=-1023 => f64::from_bits(1_u64 << (exponent + 1074)),
        -1022..=1023 => f64::from_bits(((exponent + 1023) as u64) << 52),
        _ => f64::INFINITY,
    }
}

fn sum_abs(octaves: &[OctaveInfo]) -> f64 {
    // Sequential DoubleStream.sum uses compensated summation and an uncorrected
    // sum to recover infinity when compensation arithmetic produces NaN.
    let (mut sum, mut correction, mut simple) = (0.0, 0.0, 0.0);
    for octave in octaves {
        let value = octave.amplitude.abs();
        let adjusted = value - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
        simple += value;
    }
    let result = sum - correction;
    if result.is_nan() && simple.is_infinite() {
        simple
    } else {
        result
    }
}
fn normalization_factor(sum: f64, octaves: &[OctaveInfo]) -> f64 {
    let mut variance = 0.0;
    for octave in octaves {
        let deviation = 0.270_224_783_124_521_1 * octave.amplitude.abs();
        variance += deviation * deviation;
    }
    let deviation = variance.sqrt();
    if deviation == 0.0 {
        0.0
    } else {
        (sum * (1.0 / 3.0)) / (deviation * 2.0_f64.sqrt())
    }
}
fn parity_factor(base_amplitude: f64, count: i32, modifiers: &[f64]) -> f64 {
    let (mut first, mut last) = (i32::MAX, i32::MIN);
    for index in 0..count {
        if modifier(modifiers, index as usize) != 0.0 {
            first = first.min(index);
            last = last.max(index);
        }
    }
    let distance = last.wrapping_sub(first).wrapping_add(1);
    base_amplitude * 0.5 * (1.0 / 3.0) / (0.1 * (1.0 + 1.0 / f64::from(distance)))
}
