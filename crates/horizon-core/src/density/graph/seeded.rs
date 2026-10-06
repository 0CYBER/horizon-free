use super::*;
use crate::{
    noise::NormalNoise,
    random::{
        LegacyPositionalRandom, LegacyRandom, PositionalRandom, RandomSource,
        XoroshiroPositionalRandom, XoroshiroRandom,
    },
};
use std::sync::Mutex;

#[derive(Debug)]
enum Factory {
    Legacy(LegacyPositionalRandom),
    Xoroshiro(XoroshiroPositionalRandom),
}

/// The seed-dependent portion of vanilla's RandomState compile context.
#[derive(Debug)]
pub(super) struct SeededContext {
    seed: i64,
    factory: Factory,
    instances: Mutex<HashMap<NoiseId, Arc<dyn Noise>>>,
}
impl SeededContext {
    pub(super) fn end_islands(&self) -> Arc<dyn DensitySampler> {
        Arc::new(EndIslandSampler::new(self.seed))
    }
    pub(super) fn new(seed: i64, legacy: bool) -> Self {
        let factory = if legacy {
            Factory::Legacy(LegacyRandom::new(seed).fork_positional())
        } else {
            Factory::Xoroshiro(XoroshiroRandom::new(seed).fork_positional())
        };
        Self {
            seed,
            factory,
            instances: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn noise(
        &self,
        graph: &DensityGraph,
        id: NoiseId,
    ) -> Result<Arc<dyn Noise>, CompileError> {
        let entry = graph.noise_entry(id)?;
        // Nether biome compatibility samplers are recreated on every lookup;
        // vanilla bypasses its normal named-noise instance map for these keys.
        let legacy_offset = match entry.name.as_str() {
            "minecraft:nether/temperature" => Some(0),
            "minecraft:nether/vegetation" => Some(1),
            _ => None,
        };
        if let Some(offset) = legacy_offset {
            return Ok(Arc::new(entry.definition.create_for_legacy_nether_biome(
                &mut LegacyRandom::new(self.seed.wrapping_add(offset)),
            )?));
        }
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| CompileError::PoisonedNoiseCache)?;
        if let Some(noise) = instances.get(&id) {
            return Ok(noise.clone());
        }
        let noise: Arc<dyn Noise> = match &self.factory {
            Factory::Legacy(factory) => Arc::new(
                entry
                    .definition
                    .create(&mut factory.from_hash_of(&entry.name))?,
            ),
            Factory::Xoroshiro(factory) => Arc::new(
                entry
                    .definition
                    .create(&mut factory.from_hash_of(&entry.name))?,
            ),
        };
        instances.insert(id, noise.clone());
        Ok(noise)
    }
    pub(super) fn blended(
        &self,
        definition: BlendedNoise,
    ) -> Result<Arc<dyn DensitySampler>, CompileError> {
        Ok(match &self.factory {
            Factory::Legacy(_) => Arc::new(definition.compile(&mut LegacyRandom::new(self.seed))?),
            Factory::Xoroshiro(factory) => {
                Arc::new(definition.compile(&mut factory.from_hash_of("minecraft:terrain"))?)
            }
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct NoiseEntry {
    pub(super) name: String,
    pub(super) definition: NormalNoise,
}
