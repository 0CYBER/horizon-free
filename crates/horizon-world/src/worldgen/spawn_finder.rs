//! NoiseSpawnFinder: where a new world's spawn search starts.
//!
//! NoiseBasedChunkGenerator.getOrigin samples the noise settings'
//! `spawn_target` points (SpawnTargetPoint) around the world origin and keeps
//! the block whose climate fits them best, biased towards 0 0. The world spawn
//! search (MinecraftServer.setInitialSpawn) then spirals out from its chunk.
use super::climate::{Parameter, quantize};
use horizon_core::density::{DensitySampler, SampleError, SamplerContext};
use horizon_core::jmath;
use std::sync::Arc;

const MAX_RADIUS: f32 = 2048.0;

/// One `spawn_target` entry: climate intervals over named density functions.
#[derive(Clone, Debug)]
pub struct SpawnTargetPoint {
    pub parameters: Vec<(Arc<dyn DensitySampler>, Parameter)>,
}
impl SpawnTargetPoint {
    /// SpawnTargetPoint.sampleFitness: the summed squared distances of the
    /// quantized samples to their intervals.
    pub fn sample_fitness(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<i64, SampleError> {
        let mut fitness = 0i64;
        for (function, parameter) in &self.parameters {
            let distance = parameter.distance(quantize(function.sample_value(context, position)?));
            fitness = fitness.wrapping_add(distance.wrapping_mul(distance));
        }
        Ok(fitness)
    }
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    x: i32,
    z: i32,
    fitness: i64,
}

/// NoiseSpawnFinder.findSpawnPosition: the best block column (x, z) of a
/// coarse then a fine radial search around the best of the two.
pub fn find_spawn_position(
    targets: &[SpawnTargetPoint],
    context: &mut SamplerContext,
) -> Result<[i32; 2], SampleError> {
    let mut best = fitness(targets, context, 0, 0)?;
    radial_search(targets, context, &mut best, MAX_RADIUS, 512.0)?;
    radial_search(targets, context, &mut best, 512.0, 32.0)?;
    Ok([best.x, best.z])
}

/// NoiseSpawnFinder.radialSearch: rings `increment` apart around the best
/// position so far, sampled every `increment` blocks along each ring.
fn radial_search(
    targets: &[SpawnTargetPoint],
    context: &mut SamplerContext,
    best: &mut Candidate,
    max_radius: f32,
    increment: f32,
) -> Result<(), SampleError> {
    let mut angle = 0f32;
    let mut radius = increment;
    let (origin_x, origin_z) = (best.x, best.z);
    while radius <= max_radius {
        let x = origin_x.wrapping_add((jmath::sin(f64::from(angle)) * f64::from(radius)) as i32);
        let z = origin_z.wrapping_add((jmath::cos(f64::from(angle)) * f64::from(radius)) as i32);
        let candidate = fitness(targets, context, x, z)?;
        if candidate.fitness < best.fitness {
            *best = candidate;
        }
        angle += increment / radius;
        if f64::from(angle) > std::f64::consts::PI * 2.0 {
            angle = 0.0;
            radius += increment;
        }
    }
    Ok(())
}

/// NoiseSpawnFinder.getSpawnPositionAndFitness: the best target's fitness at
/// the column's quart corner, scaled so the squared distance to 0 0 only
/// breaks ties between equally fitting columns.
fn fitness(
    targets: &[SpawnTargetPoint],
    context: &mut SamplerContext,
    x: i32,
    z: i32,
) -> Result<Candidate, SampleError> {
    let quart = [(x >> 2) << 2, 0, (z >> 2) << 2];
    let mut best = i64::MAX;
    for target in targets {
        best = best.min(target.sample_fitness(context, quart)?);
    }
    let bias = i64::from(x) * i64::from(x) + i64::from(z) * i64::from(z);
    Ok(Candidate {
        x,
        z,
        fitness: best.wrapping_mul(2048 * 2048).wrapping_add(bias),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Gradient;
    impl DensitySampler for Gradient {
        fn sample_value(&self, _: &mut SamplerContext, p: [i32; 3]) -> Result<f32, SampleError> {
            Ok(p[0] as f32 / 1000.0)
        }
    }

    #[test]
    fn the_closest_fitting_column_wins() {
        // x / 1000 must reach 0.1: every column from x = 100 on fits equally,
        // so the distance bias picks the fitting column nearest to 0 0.
        let targets = [SpawnTargetPoint {
            parameters: vec![(Arc::new(Gradient), Parameter::new(1000, 10_000))],
        }];
        let [x, z] = find_spawn_position(&targets, &mut SamplerContext::default()).unwrap();
        assert!(x >= 100, "{x} {z}");
        assert!(x * x + z * z < 160 * 160, "{x} {z}");
    }
}
